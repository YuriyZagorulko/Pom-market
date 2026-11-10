use rust_decimal::Decimal;
use serde_json::json;
use sha3::{Digest, Keccak256};

use super::RawTrade;
use crate::engine::normalize_address;

/// Standard CTF Exchange `OrderFilled` layout. Asset id 0 is collateral.
/// The worker only emits a trade when this decoder accepts the log.
pub fn order_filled_topic() -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(b"OrderFilled(bytes32,address,address,uint256,uint256,uint256,uint256,uint256)");
    hasher.finalize().into()
}

#[derive(Clone, Debug)]
pub struct DecodedFill {
    pub maker: String,
    pub maker_asset_id: u128,
    pub taker_asset_id: u128,
    pub side: &'static str,
    pub token_id: String,
    pub price: Decimal,
    pub size_shares: Decimal,
    pub tx_hash: String,
}

pub fn decode_order_filled(
    topics: &[String],
    data_hex: &str,
    tx_hash: &str,
) -> Option<DecodedFill> {
    if topics.len() < 3 {
        return None;
    }
    let topic0 = decode_hex32(topics.first()?)?;
    if topic0 != order_filled_topic() {
        return None;
    }
    let maker = topic_address(topics.get(1)?)?;
    let words = decode_words(data_hex)?;
    if words.len() != 5 {
        return None;
    }
    let maker_asset = u256_low(words[0])?;
    let taker_asset = u256_low(words[1])?;
    let maker_amount = word_decimal(words[2])?;
    let taker_amount = word_decimal(words[3])?;
    let (side, token_id, price, size) = if maker_asset == 0 && taker_asset != 0 {
        let size = scale6(taker_amount);
        let collateral = scale6(maker_amount);
        if size <= Decimal::ZERO {
            return None;
        }
        ("buy", taker_asset.to_string(), collateral / size, size)
    } else if taker_asset == 0 && maker_asset != 0 {
        let size = scale6(maker_amount);
        let collateral = scale6(taker_amount);
        if size <= Decimal::ZERO {
            return None;
        }
        ("sell", maker_asset.to_string(), collateral / size, size)
    } else {
        return None;
    };
    Some(DecodedFill {
        maker: normalize_address(&maker),
        maker_asset_id: maker_asset,
        taker_asset_id: taker_asset,
        side,
        token_id,
        price,
        size_shares: size,
        tx_hash: tx_hash.to_string(),
    })
}

pub fn decoded_to_trade(fill: DecodedFill, raw_log: serde_json::Value) -> RawTrade {
    let dedup = crate::engine::dedup_key(&fill.tx_hash, &fill.maker, &fill.token_id, fill.side);
    let fragment_key = format!("{dedup}|polygon|{}|{}", fill.price, fill.size_shares);
    RawTrade {
        dedup_key: dedup,
        fragment_key,
        source_wallet: fill.maker,
        tx_hash: Some(fill.tx_hash),
        condition_id: None,
        token_id: fill.token_id,
        outcome: None,
        market_title: None,
        market_slug: None,
        event_title: None,
        event_slug: None,
        market_icon: None,
        activity_type: "trade".into(),
        usdc_size: None,
        side: fill.side.to_string(),
        price: fill.price,
        size_shares: fill.size_shares,
        notional_usd: (fill.price * fill.size_shares).round_dp(8),
        event_ts: chrono::Utc::now(),
        origin: "polygon".into(),
        raw: raw_log,
    }
}

fn scale6(amount: Decimal) -> Decimal {
    amount / Decimal::from(1_000_000u64)
}

fn decode_hex32(value: &str) -> Option<[u8; 32]> {
    let rest = value.trim().trim_start_matches("0x");
    let bytes = hex::decode(rest).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Some(out)
}

fn topic_address(topic: &str) -> Option<String> {
    let bytes = decode_hex32(topic)?;
    Some(format!("0x{}", hex::encode(&bytes[12..])))
}

fn decode_words(data_hex: &str) -> Option<Vec<[u8; 32]>> {
    let rest = data_hex.trim().trim_start_matches("0x");
    let bytes = hex::decode(rest).ok()?;
    if bytes.len() % 32 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks(32)
            .map(|c| {
                let mut w = [0u8; 32];
                w.copy_from_slice(c);
                w
            })
            .collect(),
    )
}

fn u256_low(word: [u8; 32]) -> Option<u128> {
    if word[..16].iter().any(|b| *b != 0) {
        return None;
    }
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&word[16..]);
    Some(u128::from_be_bytes(buf))
}

fn word_decimal(word: [u8; 32]) -> Option<Decimal> {
    let low = u256_low(word)?;
    Some(Decimal::from(low))
}

pub fn synthetic_buy_log(
    maker: &str,
    token_id: u128,
    shares_6dp: u128,
    collateral_6dp: u128,
) -> serde_json::Value {
    let topic0 = format!("0x{}", hex::encode(order_filled_topic()));
    let mut maker_topic = [0u8; 32];
    let addr = hex::decode(maker.trim_start_matches("0x")).unwrap();
    maker_topic[12..].copy_from_slice(&addr);
    let mut data = Vec::new();
    data.extend(encode_u128(0));
    data.extend(encode_u128(token_id));
    data.extend(encode_u128(collateral_6dp));
    data.extend(encode_u128(shares_6dp));
    data.extend(encode_u128(0));
    json!({
        "topics": [topic0, format!("0x{}", hex::encode(maker_topic)), format!("0x{}", hex::encode([0u8; 32]))],
        "data": format!("0x{}", hex::encode(data)),
        "transactionHash": "0xabc"
    })
}

fn encode_u128(value: u128) -> [u8; 32] {
    let mut word = [0u8; 32];
    word[16..].copy_from_slice(&value.to_be_bytes());
    word
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_synthetic_collateral_buy() {
        let maker = "0x1111111111111111111111111111111111111111";
        let log = synthetic_buy_log(maker, 99, 10_000_000, 4_200_000);
        let topics = log["topics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        let decoded = decode_order_filled(&topics, log["data"].as_str().unwrap(), "0xabc").unwrap();
        assert_eq!(decoded.side, "buy");
        assert_eq!(decoded.token_id, "99");
        assert_eq!(decoded.size_shares.to_string(), "10");
        assert_eq!(decoded.price.to_string(), "0.42");
        assert_eq!(decoded.maker, maker);
    }
}
