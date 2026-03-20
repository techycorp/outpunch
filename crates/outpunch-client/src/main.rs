use std::time::Duration;

use clap::Parser;
use outpunch_client::ClientConfig;

#[derive(Parser)]
#[command(name = "outpunch-client", about = "Outpunch tunnel client")]
struct Args {
    /// WebSocket URL of the outpunch server
    #[arg(
        long,
        env = "OUTPUNCH_SERVER_URL",
        default_value = "ws://localhost:3000/ws"
    )]
    server_url: String,

    /// Shared secret for authentication
    #[arg(long, env = "OUTPUNCH_SECRET")]
    secret: String,

    /// Service name to register for
    #[arg(long, env = "OUTPUNCH_SERVICE")]
    service: String,

    /// Local URL to forward requests to
    #[arg(
        long,
        env = "OUTPUNCH_FORWARD_TO",
        default_value = "http://localhost:8080"
    )]
    forward_to: String,

    /// Seconds to wait before reconnecting
    #[arg(long, env = "OUTPUNCH_RECONNECT_DELAY", default_value = "5")]
    reconnect_delay: u64,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    eprintln!("outpunch-client");
    eprintln!("  server:     {}", args.server_url);
    eprintln!("  service:    {}", args.service);
    eprintln!("  forward_to: {}", args.forward_to);

    let config = ClientConfig {
        server_url: args.server_url,
        secret: args.secret,
        service: args.service,
        forward_to: args.forward_to,
        reconnect_delay: Duration::from_secs(args.reconnect_delay),
        ..ClientConfig::default()
    };

    outpunch_client::run(&config).await;
}
