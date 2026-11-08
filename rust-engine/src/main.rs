//! Tokio service entrypoint for the Rust Collector, Event Processor, Router, and Execution Worker daemon.
//! Exposes `/health` and `/metrics` on container port 8080 (mapped to host port 4321 in development).

use chrono::Utc;
use pommarket_engine::{parse_rtds_trade_frame, AccountRouter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let bind_addr =
        std::env::var("RUST_ENGINE_BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://pommarket:pommarket_dev_pass@localhost:5440/pommarket".to_string());
    let redis_url =
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6390/0".to_string());
    let nats_url =
        std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4225".to_string());

    let router = AccountRouter::new();
    let sample_frame = r#"{"topic":"activity","type":"trades","payload":{"proxyWallet":"0xleader1","conditionId":"0xcond1","asset":"1001","side":"BUY","price":0.52,"size":100,"timestamp":1788447208,"transactionHash":"0xabc"}}"#;
    let parsed = parse_rtds_trade_frame(sample_frame, Utc::now());
    let routed = parsed.first().map(|ev| router.route_event(ev)).unwrap_or_default();

    info!(
        bind_addr = %bind_addr,
        database_url = %database_url,
        redis_url = %redis_url,
        nats_url = %nats_url,
        parsed_events = parsed.len(),
        routed_targets = routed.len(),
        "Pom-Market Rust hot-path engine listening"
    );

    if std::env::var("RUST_ENGINE_ONESHOT").as_deref() == Ok("1") {
        return Ok(());
    }

    let listener = TcpListener::bind(&bind_addr).await?;
    loop {
        let (mut socket, _) = listener.accept().await?;
        tokio::spawn(async move {
            let mut buf = [0_u8; 1024];
            let n = socket.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let (content_type, body) = if req.starts_with("GET /metrics") {
                (
                    "text/plain; version=0.0.4",
                    "# TYPE pommarket_rust_engine_up gauge\npommarket_rust_engine_up 1\n".to_string(),
                )
            } else {
                (
                    "application/json",
                    r#"{"status":"ok","service":"pom-market-rust-engine"}"#.to_string(),
                )
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(resp.as_bytes()).await;
        });
    }
}
