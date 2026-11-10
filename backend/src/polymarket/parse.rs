use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::engine::{dedup_key, normalize_address, BookLevel, OrderBook};

#[derive(Clone, Debug)]
pub struct RawTrade {
    pub dedup_key: String,
    pub fragment_key: String,
    pub source_wallet: String,
    pub tx_hash: Option<String>,
    pub condition_id: Option<String>,
    pub token_id: String,
    pub outcome: Option<String>,
    pub market_title: Option<String>,
    pub market_slug: Option<String>,
    pub event_title: Option<String>,
    pub event_slug: Option<String>,
    pub market_icon: Option<String>,
    pub activity_type: String,
    pub usdc_size: Option<Decimal>,
    pub side: String,
    pub price: Decimal,
    pub size_shares: Decimal,
    pub notional_usd: Decimal,
    pub event_ts: DateTime<Utc>,
    pub origin: String,
    pub raw: Value,
}

pub fn parse_activity_payload(payload: &Value, origin: &str) -> Vec<RawTrade> {
    let rows = match payload {
        Value::Array(items) => items.clone(),
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get("data").or_else(|| map.get("trades")) {
                items.clone()
            } else {
                vec![payload.clone()]
            }
        }
        _ => Vec::new(),
    };
    rows.iter()
        .filter_map(|row| trade_from_value(row, origin))
        .collect()
}

fn trade_from_value(row: &Value, origin: &str) -> Option<RawTrade> {
    let kind = first_str(row, &["type", "activity_type", "event_type"]).unwrap_or_default();
    let kind_l = kind.to_ascii_lowercase();
    if matches!(
        kind_l.as_str(),
        "redeem" | "split" | "merge" | "reward" | "yield" | "conversion"
    ) {
        return None;
    }
    let side_raw = first_str(row, &["side"])?;
    let side = match side_raw.to_ascii_lowercase().as_str() {
        "buy" | "b" | "0" => "buy",
        "sell" | "s" | "1" => "sell",
        _ => return None,
    };
    let wallet = first_str(
        row,
        &["proxy_wallet", "proxyWallet", "user", "maker", "wallet"],
    )?;
    if wallet.len() < 4 {
        return None;
    }
    let token_id = first_str(
        row,
        &["token_id", "tokenId", "asset", "asset_id", "assetId"],
    )?;
    let price = first_decimal(row, &["price"])?;
    let size = first_decimal(row, &["size", "shares", "size_shares"])?;
    if price <= Decimal::ZERO || size <= Decimal::ZERO {
        return None;
    }
    let tx = first_str(
        row,
        &["transaction_hash", "transactionHash", "tx_hash", "txHash"],
    )
    .filter(|s| !s.is_empty());
    let event_ts = first_timestamp(row, &["timestamp", "match_time", "matchTime", "created_at"])?;
    let wallet_n = normalize_address(&wallet);
    let dedup = if let Some(tx) = &tx {
        dedup_key(tx, &wallet_n, &token_id, side)
    } else {
        format!(
            "notx|{wallet_n}|{token_id}|{side}|{}|{price}|{size}",
            event_ts.timestamp_millis()
        )
    };
    let fragment_key = format!("{dedup}|{origin}|{price}|{size}");
    let notional = (price * size).round_dp(8);
    Some(RawTrade {
        dedup_key: dedup,
        fragment_key,
        source_wallet: wallet_n,
        tx_hash: tx,
        condition_id: first_str(row, &["condition_id", "conditionId"]),
        token_id,
        outcome: first_str(row, &["outcome", "outcome_name"]),
        market_title: first_str(row, &["title", "market_title", "question"]),
        market_slug: first_str(row, &["slug", "market_slug"]),
        event_title: first_str(row, &["event_title", "eventTitle"]),
        event_slug: first_str(row, &["event_slug", "eventSlug"]),
        market_icon: first_str(row, &["icon", "image", "marketImage", "market_image"]),
        activity_type: first_str(row, &["type", "activity_type", "event_type"])
            .map(|t| t.to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "trade".to_string()),
        usdc_size: first_decimal(row, &["usdc_size", "usdcSize"]),
        side: side.to_string(),
        price,
        size_shares: size,
        notional_usd: notional,
        event_ts,
        origin: origin.to_string(),
        raw: row.clone(),
    })
}

pub fn parse_order_book(payload: &Value) -> Option<OrderBook> {
    let bids = levels(payload.get("bids")?);
    let asks = levels(payload.get("asks")?);
    let tick = payload
        .get("tick_size")
        .or_else(|| payload.get("tickSize"))
        .and_then(value_decimal)
        .unwrap_or(Decimal::new(1, 2));
    let min_order_size = payload
        .get("min_order_size")
        .or_else(|| payload.get("minOrderSize"))
        .and_then(value_decimal)
        .unwrap_or(Decimal::ZERO);
    Some(
        OrderBook {
            bids,
            asks,
            tick,
            min_order_size,
        }
        .normalized(),
    )
}

fn levels(value: &Value) -> Vec<BookLevel> {
    let Value::Array(items) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let (price, size) = if let Value::Object(map) = item {
                let price = map.get("price").and_then(value_decimal)?;
                let size = map
                    .get("size")
                    .or_else(|| map.get("amount"))
                    .and_then(value_decimal)?;
                (price, size)
            } else if let Value::Array(pair) = item {
                (value_decimal(pair.first()?)?, value_decimal(pair.get(1)?)?)
            } else {
                return None;
            };
            Some(BookLevel { price, size })
        })
        .collect()
}

fn first_str(row: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = row.get(*key) {
            match value {
                Value::String(s) if !s.is_empty() => return Some(s.clone()),
                Value::Number(n) => return Some(n.to_string()),
                _ => {}
            }
        }
    }
    None
}

fn first_decimal(row: &Value, keys: &[&str]) -> Option<Decimal> {
    for key in keys {
        if let Some(value) = row.get(*key) {
            if let Some(d) = value_decimal(value) {
                return Some(d);
            }
        }
    }
    None
}

fn value_decimal(value: &Value) -> Option<Decimal> {
    match value {
        Value::String(s) => Decimal::from_str_exact(s).ok().or_else(|| s.parse().ok()),
        Value::Number(n) => Decimal::from_str_exact(&n.to_string()).ok(),
        _ => None,
    }
}

fn first_timestamp(row: &Value, keys: &[&str]) -> Option<DateTime<Utc>> {
    for key in keys {
        let Some(value) = row.get(*key) else { continue };
        if let Some(ts) = parse_timestamp(value) {
            return Some(ts);
        }
    }
    None
}

fn parse_timestamp(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::Number(n) => {
            let n = n.as_i64()?;
            let secs = if n > 10_000_000_000 { n / 1000 } else { n };
            Utc.timestamp_opt(secs, 0).single()
        }
        Value::String(s) => {
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Some(dt.with_timezone(&Utc));
            }
            let n: i64 = s.parse().ok()?;
            let secs = if n > 10_000_000_000 { n / 1000 } else { n };
            Utc.timestamp_opt(secs, 0).single()
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_v2_envelope_and_camel_case_and_skips_redeem() {
        let payload = json!({
            "data": [
                {
                    "proxy_wallet": "0xAa",
                    "transaction_hash": "0xabc",
                    "token_id": "55",
                    "condition_id": "c1",
                    "side": "BUY",
                    "price": "0.42",
                    "size": "10",
                    "timestamp": 1_700_000_000,
                    "title": "Market",
                    "slug": "market"
                },
                {
                    "proxyWallet": "0xAa",
                    "transactionHash": "0xabc",
                    "asset": "55",
                    "side": "buy",
                    "price": 0.42,
                    "size": 3,
                    "timestamp": "1700000000000"
                },
                {"type": "REDEEM", "side": "SELL", "price": "1", "size": "1", "proxy_wallet": "0xAa", "token_id": "1", "timestamp": 1}
            ]
        });
        let trades = parse_activity_payload(&payload, "data_api");
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].dedup_key, trades[1].dedup_key);
        assert_ne!(trades[0].fragment_key, trades[1].fragment_key);
        assert_eq!(trades[0].side, "buy");
    }

    #[test]
    fn parses_live_v2_activity_trade() {
        let payload = json!({
            "data": [{
                "proxy_wallet": "0x4aba910be73fbaf43d2c8abdca7b8bb4533bf791",
                "timestamp": 1794175027,
                "condition_id": "0xcda676875a6c0a7f3969d2cf6ce9ac3692094a5a92f0bd563f701cf6795edde7",
                "type": "TRADE",
                "size": 7.0,
                "price": 0.739,
                "token_id": "56206431132243753610537917136876004064403943701239790116852423493128959129886",
                "side": "BUY",
                "title": "Will the price of Ethereum be above $2,100 on November 9?",
                "slug": "ethereum-above-2100-on-november-9-2026",
                "outcome": "Yes",
                "transaction_hash": "0x1bd772b6550efe975d53f9c85d1c56644833aa0debbb52ebbc08ab3e77163b13"
            }],
            "pagination": {"has_more": true, "next_cursor": "cursor"}
        });
        let trades = parse_activity_payload(&payload, "data_api");
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].side, "buy");
        assert_eq!(trades[0].outcome.as_deref(), Some("Yes"));
        assert_eq!(trades[0].size_shares.normalize().to_string(), "7");
        assert!(trades[0]
            .dedup_key
            .contains("0x1bd772b6550efe975d53f9c85d1c56644833aa0debbb52ebbc08ab3e77163b13"));
        let again = parse_activity_payload(&payload, "rtds");
        assert_eq!(trades[0].dedup_key, again[0].dedup_key);
        assert_ne!(trades[0].fragment_key, again[0].fragment_key);
    }

    #[test]
    fn parses_icon_event_slug_and_usdc_size() {
        let payload = json!([{
            "proxy_wallet": "0xAa",
            "transaction_hash": "0xicon",
            "token_id": "1",
            "side": "BUY",
            "price": "0.40",
            "size": "25",
            "usdcSize": 10,
            "timestamp": 1_700_000_000,
            "type": "TRADE",
            "title": "Q",
            "slug": "market-slug",
            "eventSlug": "event-slug",
            "icon": "https://polymarket-upload.s3.us-east-2.amazonaws.com/test.png"
        }]);
        let trades = parse_activity_payload(&payload, "data_api");
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].event_slug.as_deref(), Some("event-slug"));
        assert_eq!(trades[0].activity_type, "trade");
        assert_eq!(
            trades[0].market_icon.as_deref(),
            Some("https://polymarket-upload.s3.us-east-2.amazonaws.com/test.png")
        );
        assert_eq!(trades[0].usdc_size.unwrap().normalize().to_string(), "10");
        assert!(trades[0].event_title.is_none());
    }

    #[test]
    fn book_sorts_asks_ascending() {
        let book = parse_order_book(&json!({
            "asks": [{"price": "0.62", "size": "5"}, {"price": "0.48", "size": "8"}],
            "bids": [{"price": "0.40", "size": "2"}, {"price": "0.44", "size": "1"}],
            "tick_size": "0.01"
        }))
        .unwrap();
        assert_eq!(book.asks[0].price.to_string(), "0.48");
        assert_eq!(book.bids[0].price.to_string(), "0.44");
    }
}
