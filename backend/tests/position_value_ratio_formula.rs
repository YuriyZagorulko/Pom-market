use pommarket::copy_preview::position_value_ratio_notional;
use pommarket::engine::RejectReason;
use rust_decimal::Decimal;

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

#[test]
fn position_value_ratio_formula_examples() {
    assert_eq!(
        position_value_ratio_notional(d("200"), d("2000"), d("4000"), None).unwrap(),
        d("100")
    );
    assert_eq!(
        position_value_ratio_notional(d("100"), d("2000"), d("4000"), None).unwrap(),
        d("50")
    );
    assert_eq!(
        position_value_ratio_notional(d("200"), d("2000"), d("4000"), Some(d("2"))).unwrap(),
        d("200")
    );
}

#[test]
fn position_value_ratio_invalid_reference() {
    assert_eq!(
        position_value_ratio_notional(d("200"), d("2000"), d("0"), None),
        Err(RejectReason::SourcePositionValueZero)
    );
}
