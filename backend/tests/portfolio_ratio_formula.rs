use pommarket::copy_preview::portfolio_ratio_notional;
use rust_decimal::Decimal;

fn d(v: &str) -> Decimal {
    v.parse().unwrap()
}

#[test]
fn example_ratio_from_spec() {
    assert_eq!(
        portfolio_ratio_notional(d("4000"), d("2000"), d("20000")).unwrap(),
        d("400")
    );
    assert_eq!(
        portfolio_ratio_notional(d("1000"), d("2000"), d("20000")).unwrap(),
        d("100")
    );
    assert_eq!(
        portfolio_ratio_notional(d("500"), d("2000"), d("20000")).unwrap(),
        d("50")
    );
}
