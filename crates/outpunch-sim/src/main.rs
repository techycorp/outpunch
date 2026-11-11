//! outpunch-sim: runs an outpunch server and client as real processes, plus a stand-in local
//! service, replays a traffic file through the tunnel, and keeps every log.
//!
//! Output (in `--out`, default `target/sim`): `server.log`, `client.log`, `service.log`,
//! `sim.log`, the interleaved `all.log` (also printed live), `results.ndjson` (one line per
//! request) and `summary.txt`.

mod logs;
mod procs;
mod service;
mod summary;
mod traffic;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use tokio::sync::Mutex;

use crate::logs::{Log, Logs};
use crate::procs::{Proc, Spec};
use crate::summary::Outcome;
use crate::traffic::{Action, Event, Request, Step};

const SECRET: &str = "sim-secret";
const SERVICE: &str = "sim";

#[derive(Parser)]
#[command(
    about = "Replay a traffic file through a local outpunch server and client, keeping every log"
)]
struct Args {
    /// Traffic file: one JSON object per line (see crates/outpunch-sim/README.md)
    traffic: PathBuf,
    /// Directory for the logs and results (overwritten)
    #[arg(long, default_value = "target/sim")]
    out: PathBuf,
    /// Server binary (the axum example); `just sim` builds it
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/debug/examples/server"))]
    server_bin: PathBuf,
    /// Client binary (the outpunch-client CLI); `just sim` builds it
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/debug/outpunch-client"))]
    client_bin: PathBuf,
    /// Server: request timeout, seconds
    #[arg(long, default_value_t = 25)]
    request_timeout: u64,
    /// Server: evict a client silent this long, seconds
    #[arg(long, default_value_t = 90)]
    client_idle_timeout: u64,
    /// Client: ping interval, seconds
    #[arg(long, default_value_t = 30)]
    ping_interval: u64,
    /// Client: reconnect after hearing nothing this long, seconds
    #[arg(long, default_value_t = 75)]
    idle_timeout: u64,
    /// Keep running this long after the last step, ms (to catch reconnects and late log lines)
    #[arg(long, default_value_t = 2000)]
    tail_ms: u64,
}

struct Procs {
    server: Proc,
    client: Proc,
}

#[tokio::main]
async fn main() {
    if let Err(e) = run(Args::parse()).await {
        eprintln!("outpunch-sim: {e}");
        std::process::exit(1);
    }
}

async fn run(args: Args) -> Result<(), String> {
    let steps = std::fs::read_to_string(&args.traffic)
        .map_err(|e| format!("{}: {e}", args.traffic.display()))
        .and_then(|text| traffic::parse(&text))?;

    std::fs::create_dir_all(&args.out).map_err(|e| e.to_string())?;
    let logs = Logs::create(&args.out, Instant::now()).map_err(|e| e.to_string())?;
    let log = |name| logs.component(&args.out, name).map_err(|e| e.to_string());
    let (sim, server_log, client_log, service_log) =
        (log("sim")?, log("server")?, log("client")?, log("service")?);

    let service_port = service::start(service_log)
        .await
        .map_err(|e| e.to_string())?;
    let server_port = free_port().map_err(|e| e.to_string())?;
    sim.info(&format!(
        "{} steps from {}; service on :{service_port}, server on :{server_port}",
        steps.len(),
        args.traffic.display()
    ));

    let server = Proc::start(server_spec(&args, server_port), server_log)
        .map_err(|e| format!("{e} (run `just sim`, which builds it)"))?;
    wait_for_port(server_port).await?;
    let client = Proc::start(client_spec(&args, server_port, service_port), client_log)
        .map_err(|e| format!("{e} (run `just sim`, which builds it)"))?;
    let base = format!("http://127.0.0.1:{server_port}/tunnel/{SERVICE}");
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(args.request_timeout + 10))
        .build()
        .map_err(|e| e.to_string())?;
    wait_for_tunnel(&http, &base).await?;
    sim.info("tunnel up; replaying (at_ms counts from now)");

    let procs = Arc::new(Mutex::new(Procs { server, client }));
    let up = Instant::now();
    let outcomes = replay(steps, up, &http, &base, &procs, &sim, &logs).await;

    tokio::time::sleep(Duration::from_millis(args.tail_ms)).await;
    {
        let mut procs = procs.lock().await;
        procs.client.kill().await;
        procs.server.kill().await;
    }

    let results: String = outcomes
        .iter()
        .map(|o| serde_json::to_string(o).expect("outcome serializes") + "\n")
        .collect();
    let summary = summary::summarize(&outcomes).to_string();
    std::fs::write(args.out.join("results.ndjson"), results).map_err(|e| e.to_string())?;
    std::fs::write(args.out.join("summary.txt"), format!("{summary}\n"))
        .map_err(|e| e.to_string())?;
    sim.info(&summary);
    sim.info(&format!("logs in {}", args.out.display()));
    Ok(())
}

/// Runs every step at its time; requests run concurrently, events in their own tasks.
async fn replay(
    steps: Vec<Step>,
    up: Instant,
    http: &reqwest::Client,
    base: &str,
    procs: &Arc<Mutex<Procs>>,
    sim: &Log,
    logs: &Logs,
) -> Vec<Outcome> {
    let tasks: Vec<_> = steps
        .into_iter()
        .map(|step| {
            let (http, base, procs, sim, logs) = (
                http.clone(),
                base.to_string(),
                procs.clone(),
                sim.clone(),
                logs.clone(),
            );
            tokio::spawn(async move {
                tokio::time::sleep_until((up + Duration::from_millis(step.at_ms)).into()).await;
                match step.action {
                    Action::Request(request) => {
                        Some(send(&http, &base, step.line, request, &sim, &logs).await)
                    }
                    Action::Event(event) => {
                        apply(event, &procs, &sim).await;
                        None
                    }
                }
            })
        })
        .collect();
    let mut outcomes = Vec::new();
    for task in tasks {
        if let Ok(Some(outcome)) = task.await {
            outcomes.push(outcome);
        }
    }
    outcomes.sort_by_key(|o| (o.sent_ms, o.line));
    outcomes
}

async fn send(
    http: &reqwest::Client,
    base: &str,
    line: usize,
    request: Request,
    sim: &Log,
    logs: &Logs,
) -> Outcome {
    let method =
        reqwest::Method::from_bytes(request.method.as_bytes()).unwrap_or(reqwest::Method::GET);
    let mut builder = http.request(method, format!("{base}{}", request.path));
    for (name, value) in &request.headers {
        builder = builder.header(name, value);
    }
    let reply = &request.reply;
    if let Some(status) = reply.status {
        builder = builder.header(service::STATUS, status.to_string());
    }
    if let Some(delay) = reply.delay_ms {
        builder = builder.header(service::DELAY_MS, delay.to_string());
    }
    if let Some(body) = &reply.body {
        builder = builder.header(service::BODY, body);
    }
    if let Some(body) = request.body.clone() {
        builder = builder.body(body);
    }

    let sent_ms = logs.elapsed_ms();
    let started = Instant::now();
    let result = builder.send().await;
    let latency_ms = started.elapsed().as_millis() as u64;
    let (status, error) = match result {
        Ok(response) => (Some(response.status().as_u16()), None),
        Err(e) => (None, Some(e.to_string())),
    };
    let failed = status.is_none_or(|s| s >= 500);
    let report = if failed { Log::warn } else { Log::info };
    report(
        sim,
        &format!(
            "line {line}: {} {} -> {} in {latency_ms} ms",
            request.method,
            request.path,
            status.map_or_else(
                || format!("error ({})", error.as_deref().unwrap_or("")),
                |s| s.to_string()
            )
        ),
    );
    Outcome {
        line,
        method: request.method,
        path: request.path,
        sent_ms,
        status,
        latency_ms,
        error,
    }
}

async fn apply(event: Event, procs: &Arc<Mutex<Procs>>, sim: &Log) {
    match event {
        Event::ClientPause { ms } => {
            sim.info(&format!("event: pausing the client for {ms} ms (SIGSTOP)"));
            let _ = procs.lock().await.client.signal("STOP").await;
            tokio::time::sleep(Duration::from_millis(ms)).await;
            let _ = procs.lock().await.client.signal("CONT").await;
            sim.info("event: client resumed (SIGCONT)");
        }
        Event::ClientRestart => {
            sim.info("event: killing and restarting the client");
            if let Err(e) = procs.lock().await.client.restart().await {
                sim.info(&format!("event: client restart failed: {e}"));
            }
        }
        Event::ServerRestart => {
            sim.info("event: killing and restarting the server");
            if let Err(e) = procs.lock().await.server.restart().await {
                sim.info(&format!("event: server restart failed: {e}"));
            }
        }
    }
}

fn server_spec(args: &Args, port: u16) -> Spec {
    Spec {
        program: args.server_bin.clone(),
        args: vec![],
        env: vec![
            ("PORT".into(), port.to_string()),
            ("OUTPUNCH_SECRET".into(), SECRET.into()),
            ("OUTPUNCH_TIMEOUT".into(), args.request_timeout.to_string()),
            (
                "OUTPUNCH_CLIENT_IDLE_TIMEOUT".into(),
                args.client_idle_timeout.to_string(),
            ),
        ],
    }
}

fn client_spec(args: &Args, server_port: u16, service_port: u16) -> Spec {
    Spec {
        program: args.client_bin.clone(),
        args: vec![
            format!("--server-url=ws://127.0.0.1:{server_port}/ws"),
            format!("--secret={SECRET}"),
            format!("--service={SERVICE}"),
            format!("--forward-to=http://127.0.0.1:{service_port}"),
            format!("--ping-interval={}", args.ping_interval),
            format!("--idle-timeout={}", args.idle_timeout),
        ],
        env: vec![],
    }
}

fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}

async fn wait_for_port(port: u16) -> Result<(), String> {
    for _ in 0..200 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(format!("the server did not listen on :{port} within 10 s"))
}

/// The tunnel is up once a request through it stops getting 502 (no client registered).
async fn wait_for_tunnel(http: &reqwest::Client, base: &str) -> Result<(), String> {
    for _ in 0..300 {
        if let Ok(response) = http.get(format!("{base}/__sim_ready")).send().await
            && response.status() != reqwest::StatusCode::BAD_GATEWAY
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err("the client did not register within 30 s; see client.log".into())
}
