//! Documentation and pure helpers for source-trade deduplication (audit reference).

/// `fragment_key` = `{dedup_key}|{price}|{size}` (see `parse::trade_from_value`).
/// `dedup_key` with tx = `{tx}|{wallet}|{token}|{side}`; without tx includes ts+price+size.
#[allow(dead_code)]
pub fn describe_fragment_key(dedup: &str, price: &str, size: &str) -> String {
    format!("{dedup}|{price}|{size}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::dedup_key;
    use crate::polymarket::parse_activity_payload;
    use serde_json::json;

    /// Scenario B (parse-level): distinct tx → distinct dedup even if price/size/time match.
    #[test]
    fn distinct_tx_never_share_dedup_key() {
        let ts = 1_700_000_000_i64;
        let mk = |tx: &str| {
            json!({
                "proxy_wallet": "0xAa",
                "transaction_hash": tx,
                "token_id": "99",
                "side": "BUY",
                "price": "0.5",
                "size": "10",
                "timestamp": ts
            })
        };
        let a = parse_activity_payload(&mk("0xtx1"), "data_api")[0].clone();
        let b = parse_activity_payload(&mk("0xtx2"), "data_api")[0].clone();
        assert_ne!(a.dedup_key, b.dedup_key);
        assert_ne!(a.fragment_key, b.fragment_key);
    }

    /// Scenario A (parse-level): same payload, different origin → same keys.
    #[test]
    fn data_api_and_rtds_share_fragment_key() {
        let row = json!({
            "proxy_wallet": "0xBb",
            "transaction_hash": "0xshared",
            "token_id": "1",
            "side": "BUY",
            "price": "0.89",
            "size": "1000",
            "timestamp": 1_700_000_001,
            "usdcSize": 890
        });
        let api = &parse_activity_payload(&row, "data_api")[0];
        let rtds = &parse_activity_payload(&row, "rtds")[0];
        assert_eq!(api.fragment_key, rtds.fragment_key);
        assert_eq!(api.dedup_key, rtds.dedup_key);
        assert_ne!(api.origin, rtds.origin);
    }

    /// Scenario E: side is part of dedup_key.
    #[test]
    fn buy_and_sell_diverge() {
        let tx = "0xsame";
        let base = |side: &str| {
            json!({
                "proxy_wallet": "0xCc",
                "transaction_hash": tx,
                "token_id": "7",
                "side": side,
                "price": "0.6",
                "size": "5",
                "timestamp": 1
            })
        };
        let buy = &parse_activity_payload(&base("BUY"), "data_api")[0];
        let sell = &parse_activity_payload(&base("SELL"), "data_api")[0];
        assert_ne!(buy.dedup_key, sell.dedup_key);
        assert_eq!(buy.dedup_key, dedup_key(tx, "0xCc", "7", "buy"));
        assert_eq!(sell.dedup_key, dedup_key(tx, "0xCc", "7", "sell"));
    }

    /// Scenario D: same dedup, different size → different fragment_key.
    #[test]
    fn partial_fills_different_fragment_keys() {
        let tx = "0xpartial";
        let mk = |size: &str| {
            json!({
                "proxy_wallet": "0xDd",
                "transaction_hash": tx,
                "token_id": "3",
                "side": "buy",
                "price": "0.42",
                "size": size,
                "timestamp": 1_700_000_000
            })
        };
        let a = &parse_activity_payload(&mk("10"), "data_api")[0];
        let b = &parse_activity_payload(&mk("3"), "data_api")[0];
        assert_eq!(a.dedup_key, b.dedup_key);
        assert_ne!(a.fragment_key, b.fragment_key);
    }

    #[test]
    fn fragment_key_format_matches_helper() {
        let t = &parse_activity_payload(
            &json!({
                "proxy_wallet": "0xEe",
                "transaction_hash": "0xf",
                "token_id": "1",
                "side": "buy",
                "price": "0.5",
                "size": "2",
                "timestamp": 1
            }),
            "data_api",
        )[0];
        let expected = describe_fragment_key(&t.dedup_key, &t.price.to_string(), &t.size_shares.to_string());
        assert_eq!(t.fragment_key, expected);
    }
}
