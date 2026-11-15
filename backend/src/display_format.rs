//! Human-readable number formatting for API/UI copy (not ledger precision).

use rust_decimal::Decimal;

/// USD for UI: thousands separators, up to 2 decimals, trim trailing zeros except sub-dollar needs cents.
pub fn fmt_usd(d: Decimal) -> String {
    let v = d.round_dp(2);
    let abs = v.abs();
    let sign = if v.is_sign_negative() { "-" } else { "" };
    let (int, frac) = split_decimal(abs);
    let int_fmt = add_thousands(&int);
    if frac.is_empty() || frac == "00" {
        format!("{sign}${int_fmt}")
    } else {
        let frac_trim = frac.trim_end_matches('0');
        let frac_show = if frac_trim.is_empty() { "00" } else { frac_trim };
        if abs < Decimal::ONE && !frac_show.is_empty() {
            format!("{sign}${int_fmt}.{frac_show}")
        } else if frac_show.len() <= 2 {
            format!("{sign}${int_fmt}.{frac_show}")
        } else {
            format!("{sign}${int_fmt}.{frac_show}")
        }
    }
}

/// Multipliers / coefficients (no `$`).
pub fn fmt_coef(d: Decimal) -> String {
    let v = d.normalize();
    let s = v.to_string();
    trim_decimal_string(&s)
}

fn split_decimal(abs: Decimal) -> (String, String) {
    let s = abs.round_dp(8).normalize().to_string();
    if let Some((a, b)) = s.split_once('.') {
        (a.to_string(), b.to_string())
    } else {
        (s, String::new())
    }
}

fn add_thousands(int: &str) -> String {
    let chars: Vec<char> = int.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        let from_right = chars.len() - i;
        if from_right != chars.len() && from_right % 3 == 0 {
            out.push(',');
        }
        out.push(*c);
    }
    out
}

fn trim_decimal_string(s: &str) -> String {
    if let Some((a, b)) = s.split_once('.') {
        let b = b.trim_end_matches('0');
        if b.is_empty() {
            a.to_string()
        } else {
            format!("{a}.{b}")
        }
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn usd_thousands_and_cents() {
        assert_eq!(fmt_usd(d("2000")), "$2,000");
        assert_eq!(fmt_usd(d("612906.98")), "$612,906.98");
        assert_eq!(fmt_usd(d("0.44")), "$0.44");
        assert_eq!(fmt_usd(d("13.51")), "$13.51");
    }

    #[test]
    fn coef_strips_zeros() {
        assert_eq!(fmt_coef(d("10")), "10");
        assert_eq!(fmt_coef(d("1.5")), "1.5");
        assert_eq!(fmt_coef(d("0.25")), "0.25");
    }
}
