//! The server and client child processes. Their stderr and stdout are captured line by line into
//! their logs (stdout lines are marked `[stdout]`; outpunch writes to stderr).

use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::{Child, Command};

use crate::logs::Log;

/// How to start one child process again after it is killed.
pub struct Spec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

pub struct Proc {
    spec: Spec,
    log: Log,
    child: Child,
}

impl Proc {
    pub fn start(spec: Spec, log: Log) -> std::io::Result<Self> {
        let child = spawn(&spec, &log)?;
        Ok(Self { spec, log, child })
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// SIGKILL: the process ends without closing its sockets cleanly.
    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }

    pub async fn restart(&mut self) -> std::io::Result<()> {
        self.kill().await;
        self.child = spawn(&self.spec, &self.log)?;
        Ok(())
    }

    /// SIGSTOP or SIGCONT through `kill(1)`.
    pub async fn signal(&self, signal: &str) -> std::io::Result<()> {
        let Some(pid) = self.pid() else {
            return Ok(());
        };
        Command::new("kill")
            .args([format!("-{signal}"), pid.to_string()])
            .status()
            .await
            .map(|_| ())
    }
}

fn spawn(spec: &Spec, log: &Log) -> std::io::Result<Child> {
    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", spec.program.display())))?;
    if let Some(stderr) = child.stderr.take() {
        pump(stderr, log.clone(), "");
    }
    if let Some(stdout) = child.stdout.take() {
        pump(stdout, log.clone(), "[stdout] ");
    }
    Ok(child)
}

fn pump(stream: impl AsyncRead + Unpin + Send + 'static, log: Log, mark: &'static str) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            log.line(&format!("{mark}{line}"));
        }
    });
}
