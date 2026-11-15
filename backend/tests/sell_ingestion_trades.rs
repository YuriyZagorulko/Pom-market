//! Data API `/v2/trades` includes SELL rows that `/v2/activity` may omit for busy wallets.

use pommarket::polymarket::parse_activity_payload;
use serde_json::json;

#[test]
fn trades_payload_parses_buy_and_sell() {
    let payload = json!({
        "data": [
            {
                "proxy_wallet": "0xd9e0aaca471f489be338fd0f91a26e8669a805f2",
                "side": "SELL",
                "token_id": "1",
                "price": 0.5,
                "size": 10,
                "timestamp": 1_700_000_000,
                "transaction_hash": "0xselltx"
            },
            {
                "proxy_wallet": "0xd9e0aaca471f489be338fd0f91a26e8669a805f2",
                "side": "BUY",
                "token_id": "2",
                "price": 0.4,
                "size": 5,
                "timestamp": 1_700_000_001,
                "transaction_hash": "0xbuytx"
            }
        ],
        "pagination": {"next_cursor": "abc", "has_more": true}
    });
    let trades = parse_activity_payload(&payload, "data_api");
    assert_eq!(trades.len(), 2);
    assert_eq!(trades[0].side, "sell");
    assert_eq!(trades[1].side, "buy");
    assert_ne!(trades[0].dedup_key, trades[1].dedup_key);
}

#[test]
fn redeem_stays_out_of_trade_stream() {
    let payload = json!([{
        "type": "REDEEM",
        "proxy_wallet": "0xaa",
        "token_id": "1",
        "side": "SELL",
        "price": 1,
        "size": 1,
        "timestamp": 1
    }]);
    assert!(parse_activity_payload(&payload, "data_api").is_empty());
}
