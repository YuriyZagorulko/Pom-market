//! Canonical leader trade amounts for copy sizing and activity display.

use rust_decimal::Decimal;

/// Prefer `usdc_size` (Polymarket cash leg) when present; avoids inflated `size_shares`
/// after duplicate fragments from multiple ingest origins.
pub fn canonical_source_trade(
    price: Decimal,
    size_shares: Decimal,
    notional_usd: Decimal,
    usdc_size: Option<Decimal>,
) -> (Decimal, Decimal) {
    if let Some(usdc) = usdc_size.filter(|u| *u > Decimal::ZERO) {
        let shares = if price > Decimal::ZERO {
            (usdc / price).round_dp(8)
        } else {
            size_shares
        };
        return (shares, usdc);
    }
    (size_shares, notional_usd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn prefers_usdc_over_doubled_shares() {
        let (shares, notional) = canonical_source_trade(d("0.89"), d("2000"), d("1780"), Some(d("890")));
        assert_eq!(shares, d("1000"));
        assert_eq!(notional, d("890"));
    }

    #[test]
    fn falls_back_to_stored_fields() {
        let (shares, notional) = canonical_source_trade(d("0.739"), d("7"), d("5.173"), None);
        assert_eq!(shares, d("7"));
        assert_eq!(notional, d("5.173"));
    }

    #[test]
    fn usdc_path_matches_price_times_shares() {
        let price = d("0.89");
        let usdc = d("890");
        let (shares, notional) = canonical_source_trade(price, d("2000"), d("1780"), Some(usdc));
        let product = (shares * price).round_dp(2);
        assert_eq!(notional, usdc);
        assert!((product - usdc).abs() <= d("0.01"));
    }

    /// Legacy rows: inflated size_shares + trustworthy usdc_size (fragment_count=2 case).
    #[test]
    fn legacy_doubled_shares_corrected_via_usdc() {
        let (shares, notional) =
            canonical_source_trade(d("0.69"), d("1000"), d("690"), Some(d("345")));
        assert_eq!(shares, d("500"));
        assert_eq!(notional, d("345"));
        assert!((shares * d("0.69") - notional).abs() <= d("0.01"));
    }
}
