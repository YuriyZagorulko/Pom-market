use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};

use super::{parse_activity_payload, RawTrade};

pub fn subscribe_message() -> String {
    json!({
        "action": "subscribe",
        "subscriptions": [
            {"topic": "activity", "type": "trades"},
            {"topic": "activity", "type": "orders_matched"}
        ]
    })
    .to_string()
}

pub fn trades_from_frame(text: &str) -> Vec<RawTrade> {
    if text.trim() == "pong" || text.trim() == "ping" {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let topic = value.get("topic").and_then(|v| v.as_str()).unwrap_or("");
    let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if topic == "activity" && !matches!(kind, "trades" | "orders_matched" | "") {
        return Vec::new();
    }
    let payload = value
        .get("payload")
        .or_else(|| value.get("data"))
        .unwrap_or(&value);
    parse_activity_payload(payload, "rtds")
}

pub fn filter_wallets(trades: Vec<RawTrade>, watched: &[String]) -> Vec<RawTrade> {
    if watched.is_empty() {
        return Vec::new();
    }
    trades
        .into_iter()
        .filter(|trade| watched.iter().any(|wallet| wallet == &trade.source_wallet))
        .collect()
}

pub fn backoff_with_jitter(attempt: u64) -> Duration {
    let base = (1u64 << attempt.min(5)).min(30);
    let jitter = attempt.wrapping_mul(17) % 5;
    Duration::from_secs(base + jitter)
}

pub async fn run_rtds<F, Fut>(url: String, mut on_trade: F)
where
    F: FnMut(Vec<RawTrade>) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let mut backoff = 1u64;
    loop {
        match connect_async(&url).await {
            Ok((mut socket, _)) => {
                info!(%url, "rtds connected");
                backoff = 1;
                if socket
                    .send(Message::Text(subscribe_message().into()))
                    .await
                    .is_err()
                {
                    warn!("rtds subscribe failed");
                    continue;
                }
                let mut last_frame = tokio::time::Instant::now();
                loop {
                    if last_frame.elapsed() > Duration::from_secs(30) {
                        warn!("rtds stream went quiet; reconnecting and backfilling through the data api");
                        break;
                    }
                    match tokio::time::timeout(Duration::from_secs(5), socket.next()).await {
                        Err(_) => {
                            if socket.send(Message::Text("ping".into())).await.is_err() {
                                break;
                            }
                        }
                        Ok(None) => break,
                        Ok(Some(Ok(Message::Text(text)))) => {
                            last_frame = tokio::time::Instant::now();
                            let trades = trades_from_frame(&text);
                            if !trades.is_empty() {
                                on_trade(trades).await;
                            }
                        }
                        Ok(Some(Ok(Message::Ping(payload)))) => {
                            last_frame = tokio::time::Instant::now();
                            let _ = socket.send(Message::Pong(payload)).await;
                        }
                        Ok(Some(Ok(Message::Close(_)))) => break,
                        Ok(Some(Err(err))) => {
                            warn!(error = %crate::security::redact(&err.to_string()), "rtds read error");
                            break;
                        }
                        Ok(Some(Ok(_))) => {}
                    }
                }
            }
            Err(err) => {
                warn!(error = %crate::security::redact(&err.to_string()), "rtds connect failed");
            }
        }
        sleep(backoff_with_jitter(backoff)).await;
        backoff = (backoff + 1).min(5);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_locally_by_proxy_wallet() {
        let frame = r#"{"topic":"activity","type":"trades","payload":{"proxyWallet":"0xAa","asset":"55","conditionId":"c","side":"BUY","price":"0.4","size":"2","timestamp":1700000000,"transactionHash":"0xabc","outcome":"Yes"}}"#;
        let trades = trades_from_frame(frame);
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].token_id, "55");
        let kept = filter_wallets(trades.clone(), &["0xaa".into()]);
        assert_eq!(kept.len(), 1);
        assert!(filter_wallets(trades, &["0xbb".into()]).is_empty());
    }

    #[test]
    fn backoff_is_bounded() {
        assert!(backoff_with_jitter(1) <= Duration::from_secs(35));
        assert!(backoff_with_jitter(8) <= Duration::from_secs(35));
    }
}
