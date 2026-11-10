use std::time::Duration;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{json, Value};

use super::{parse_activity_payload, parse_order_book, RawTrade};
use crate::engine::OrderBook;
use crate::security::redact;

#[derive(Clone)]
pub struct PublicClient {
    http: reqwest::Client,
    data_api: String,
    clob: String,
    gamma: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PublicProfile {
    pub wallet: String,
    pub positions: usize,
    pub value_usd: Option<Decimal>,
    pub last_trade_at: Option<DateTime<Utc>>,
    pub sample_titles: Vec<String>,
    pub error: Option<String>,
}

impl PublicClient {
    pub fn new(data_api: String, clob: String, gamma: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("pommarket-copy-platform")
            .build()
            .expect("http client");
        Self {
            http,
            data_api,
            clob,
            gamma,
        }
    }

    async fn send_with_retry(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, String> {
        let mut attempt = 0u32;
        loop {
            let prepared = request
                .try_clone()
                .ok_or_else(|| "data api request could not be retried".to_string())?;
            let response = prepared.send().await.map_err(|e| redact(&e.to_string()))?;
            let code = response.status().as_u16();
            if code != 429 && code != 503 || attempt >= 3 {
                return Ok(response);
            }
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(1)
                .min(10);
            tokio::time::sleep(Duration::from_secs(retry_after)).await;
            attempt += 1;
        }
    }

    pub async fn clob_ok(&self) -> Result<bool, String> {
        let url = format!("{}/ok", self.clob.trim_end_matches('/'));
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| redact(&e.to_string()))?;
        Ok(response.status().is_success())
    }

    pub async fn order_book(&self, token_id: &str) -> Result<OrderBook, String> {
        let url = format!("{}/book", self.clob.trim_end_matches('/'));
        let response = self
            .http
            .get(url)
            .query(&[("token_id", token_id)])
            .send()
            .await
            .map_err(|e| redact(&e.to_string()))?;
        if !response.status().is_success() {
            return Err(format!("order book HTTP {}", response.status()));
        }
        let body: Value = response.json().await.map_err(|e| redact(&e.to_string()))?;
        parse_order_book(&body).ok_or_else(|| "order book response was not usable".into())
    }

    pub async fn fee_schedule(&self, token_id: &str) -> Option<crate::engine::FeeSchedule> {
        let market = self.market_by_token(token_id).await?;
        parse_market_fee(&market)
    }

    pub async fn activity_page(
        &self,
        wallet: &str,
        cursor: Option<&str>,
    ) -> Result<(Vec<RawTrade>, Option<String>), String> {
        let base = self.data_api.trim_end_matches('/');
        let url = format!("{base}/v2/activity");
        let mut request = self
            .http
            .get(url)
            .query(&[("user", wallet), ("limit", "100")]);
        if let Some(cursor) = cursor {
            request = request.query(&[("cursor", cursor)]);
        }
        let response = self.send_with_retry(request).await?;
        if response.status().as_u16() == 410 || response.status().as_u16() == 404 {
            return self.activity_fallback(wallet).await;
        }
        if !response.status().is_success() {
            return Err(format!("data api HTTP {}", response.status()));
        }
        let body: Value = response.json().await.map_err(|e| redact(&e.to_string()))?;
        let trades = parse_activity_payload(&body, "data_api");
        let next = body
            .pointer("/pagination/next_cursor")
            .or_else(|| body.get("next_cursor"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        Ok((trades, next))
    }

    async fn activity_fallback(
        &self,
        wallet: &str,
    ) -> Result<(Vec<RawTrade>, Option<String>), String> {
        let url = format!("{}/v2/trades", self.data_api.trim_end_matches('/'));
        let response = self
            .http
            .get(url)
            .query(&[("user", wallet), ("limit", "100")])
            .send()
            .await
            .map_err(|e| redact(&e.to_string()))?;
        if !response.status().is_success() {
            return Err(format!("data api trades HTTP {}", response.status()));
        }
        let body: Value = response.json().await.map_err(|e| redact(&e.to_string()))?;
        Ok((parse_activity_payload(&body, "data_api"), None))
    }

    pub async fn profile_for_registration(&self, wallet: &str) -> PublicProfile {
        match tokio::time::timeout(Duration::from_secs(8), self.profile(wallet)).await {
            Ok(profile) => profile,
            Err(_) => PublicProfile {
                wallet: wallet.to_string(),
                positions: 0,
                value_usd: None,
                last_trade_at: None,
                sample_titles: Vec::new(),
                error: Some(
                    "initial Polymarket profile sync timed out; wallet was added and monitoring can continue"
                        .into(),
                ),
            },
        }
    }

    pub async fn profile(&self, wallet: &str) -> PublicProfile {
        let positions_url = format!("{}/v2/positions", self.data_api.trim_end_matches('/'));
        let value_url = format!("{}/v2/value", self.data_api.trim_end_matches('/'));
        let positions: Value = match self
            .http
            .get(&positions_url)
            .query(&[("user", wallet)])
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => resp.json().await.unwrap_or(json!([])),
            Ok(resp) => {
                return PublicProfile {
                    wallet: wallet.to_string(),
                    positions: 0,
                    value_usd: None,
                    last_trade_at: None,
                    sample_titles: Vec::new(),
                    error: Some(format!("positions HTTP {}", resp.status())),
                };
            }
            Err(err) => {
                return PublicProfile {
                    wallet: wallet.to_string(),
                    positions: 0,
                    value_usd: None,
                    last_trade_at: None,
                    sample_titles: Vec::new(),
                    error: Some(redact(&err.to_string())),
                };
            }
        };
        let rows = positions
            .get("data")
            .and_then(|value| value.as_array())
            .cloned()
            .or_else(|| positions.as_array().cloned())
            .unwrap_or_default();
        let titles = rows
            .iter()
            .filter_map(|row| {
                row.get("title")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .take(5)
            .collect();
        let value_usd = match self
            .http
            .get(&value_url)
            .query(&[("user", wallet)])
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                resp.json::<Value>().await.ok().and_then(|v| {
                    let node = v.get("data").unwrap_or(&v);
                    node.get("value").and_then(json_decimal)
                })
            }
            _ => None,
        };
        let (trades, _) = self
            .activity_page(wallet, None)
            .await
            .unwrap_or((Vec::new(), None));
        PublicProfile {
            wallet: wallet.to_string(),
            positions: rows.len(),
            value_usd,
            last_trade_at: trades.iter().map(|t| t.event_ts).max(),
            sample_titles: titles,
            error: None,
        }
    }

    pub async fn market_by_token(&self, token_id: &str) -> Option<Value> {
        let url = format!("{}/markets", self.gamma.trim_end_matches('/'));
        let response = self
            .http
            .get(url)
            .query(&[("clob_token_ids", token_id), ("limit", "1")])
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let body: Value = response.json().await.ok()?;
        body.as_array().and_then(|rows| rows.first().cloned())
    }

    pub async fn polygon_logs(
        &self,
        rpc: &str,
        address: &str,
        topic0: &str,
        from_block: u64,
        to_block: u64,
    ) -> Result<Vec<Value>, String> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_getLogs",
            "params": [{
                "address": address,
                "fromBlock": format!("0x{from_block:x}"),
                "toBlock": format!("0x{to_block:x}"),
                "topics": [topic0]
            }]
        });
        let response = self
            .http
            .post(rpc)
            .json(&body)
            .send()
            .await
            .map_err(|e| redact(&e.to_string()))?;
        let payload: Value = response.json().await.map_err(|e| redact(&e.to_string()))?;
        if let Some(error) = payload.get("error") {
            return Err(redact(&error.to_string()));
        }
        Ok(payload
            .get("result")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    /// `GET /v2/value` — open position mark value (excludes free collateral).
    pub async fn portfolio_value_usd(&self, proxy_wallet: &str) -> Result<Decimal, String> {
        let url = format!("{}/v2/value", self.data_api.trim_end_matches('/'));
        let response = self
            .http
            .get(&url)
            .query(&[("user", proxy_wallet)])
            .send()
            .await
            .map_err(|e| redact(&e.to_string()))?;
        if !response.status().is_success() {
            return Err(format!("data api value HTTP {}", response.status()));
        }
        let body: Value = response.json().await.map_err(|e| redact(&e.to_string()))?;
        let node = body.get("data").unwrap_or(&body);
        node.get("value")
            .and_then(json_decimal)
            .ok_or_else(|| "value field missing".into())
    }

    /// Sum `current_value` on combo positions when present.
    pub async fn combo_positions_value_usd(&self, proxy_wallet: &str) -> Result<Decimal, String> {
        let url = format!(
            "{}/v2/positions/combos",
            self.data_api.trim_end_matches('/')
        );
        let response = self
            .http
            .get(&url)
            .query(&[("user", proxy_wallet), ("limit", "100")])
            .send()
            .await
            .map_err(|e| redact(&e.to_string()))?;
        if !response.status().is_success() {
            return Err(format!("data api combos HTTP {}", response.status()));
        }
        let body: Value = response.json().await.map_err(|e| redact(&e.to_string()))?;
        let rows = body
            .get("data")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let sum = rows
            .iter()
            .filter_map(|row| row.get("current_value").and_then(json_decimal))
            .fold(Decimal::ZERO, |a, b| a + b);
        Ok(sum.round_dp(8))
    }
}

pub fn parse_market_fee(market: &Value) -> Option<crate::engine::FeeSchedule> {
    let enabled = market.get("feesEnabled").and_then(|v| v.as_bool());
    let schedule = market.get("feeSchedule")?;
    let rate = schedule.get("rate").and_then(json_decimal)?;
    let exponent = schedule
        .get("exponent")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;
    let taker_only = schedule
        .get("takerOnly")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let rebate_rate = schedule.get("rebateRate").and_then(json_decimal);
    Some(crate::engine::FeeSchedule {
        fees_enabled: enabled.unwrap_or(true),
        rate,
        exponent,
        taker_only,
        rebate_rate,
        source: "gamma.feeSchedule".into(),
    })
}

pub fn fee_rate_from_raw(raw: Decimal) -> Decimal {
    if raw > Decimal::ONE {
        raw / Decimal::from(10_000)
    } else {
        raw
    }
}

fn json_decimal(value: &Value) -> Option<Decimal> {
    match value {
        Value::String(text) => text.parse().ok(),
        Value::Number(number) => number.to_string().parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_fee_bps_are_not_used_as_a_multiplier() {
        assert_eq!(
            fee_rate_from_raw(Decimal::from(1000))
                .normalize()
                .to_string(),
            "0.1"
        );
        let market = json!({
            "feesEnabled": true,
            "feeSchedule": {"exponent": 1, "rate": 0.07, "takerOnly": true, "rebateRate": 0.2}
        });
        let parsed = parse_market_fee(&market).unwrap();
        assert_eq!(parsed.rate, Decimal::new(7, 2));
        assert!(parsed.taker_only);
        assert!(parse_market_fee(&json!({"feesEnabled": true})).is_none());
    }
}
