use pommarket::polymarket::http::{MarketResolutionKind, resolution_for_token};
use rust_decimal::Decimal;
use serde_json::json;

#[test]
fn resolved_win_pays_one_per_share() {
    let market = json!({
        "closed": true,
        "tokens": [
            {"token_id": "1", "outcome": "MOUZ", "winner": false, "price": 0},
            {"token_id": "2", "outcome": "G2", "winner": true, "price": 1}
        ]
    });
    let r = resolution_for_token(&market, "2", Some("G2"));
    assert_eq!(r.kind, MarketResolutionKind::Win);
    assert_eq!(r.payout_per_share, Decimal::ONE);
}

#[test]
fn resolved_loss_pays_zero() {
    let market = json!({
        "closed": true,
        "tokens": [
            {"token_id": "1", "outcome": "MOUZ", "winner": false, "price": 0},
            {"token_id": "2", "outcome": "G2", "winner": true, "price": 1}
        ]
    });
    let r = resolution_for_token(&market, "1", Some("MOUZ"));
    assert_eq!(r.kind, MarketResolutionKind::Loss);
    assert_eq!(r.payout_per_share, Decimal::ZERO);
}

#[test]
fn active_market_not_settled() {
    let market = json!({
        "closed": false,
        "tokens": [{"token_id": "1", "outcome": "Yes", "winner": false, "price": 0.5}]
    });
    let r = resolution_for_token(&market, "1", Some("Yes"));
    assert_eq!(r.kind, MarketResolutionKind::Active);
}
