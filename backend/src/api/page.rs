//! List query parsing. Date-only `from`/`to` values use the **UTC calendar** (`YYYY-MM-DD`).
//! `from` is inclusive at 00:00:00 UTC; date-only `to` is inclusive through that day
//! (SQL upper bound is start of the next UTC day with `<`).
//! Full RFC3339 `to` timestamps are used as an exclusive upper bound unchanged.

use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::Serialize;
use std::str::FromStr;
use uuid::Uuid;

#[derive(serde::Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub page: Option<i64>,
    pub q: Option<String>,
    pub side: Option<String>,
    pub status: Option<String>,
    pub mode: Option<String>,
    pub account_id: Option<Uuid>,
    pub wallets: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub min_amount: Option<String>,
    pub max_amount: Option<String>,
    pub min_price: Option<String>,
    pub max_price: Option<String>,
    pub outcome: Option<String>,
    pub kind: Option<String>,
    pub activity_type: Option<String>,
}

#[derive(Serialize)]
pub struct Paged<T> {
    pub items: T,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

impl ListQuery {
    pub fn limit(&self) -> i64 {
        self.limit.unwrap_or(25).clamp(1, 100)
    }

    pub fn offset(&self) -> i64 {
        if let Some(page) = self.page {
            return ((page.max(1) - 1) * self.limit()).max(0);
        }
        self.offset.unwrap_or(0).max(0)
    }

    pub fn filter_text(value: &Option<String>) -> Option<String> {
        Self::filter_text_opt(value)
    }

    pub fn filter_text_opt(value: &Option<String>) -> Option<String> {
        value
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    pub fn filter_side(&self) -> Option<String> {
        Self::filter_text(&self.side).map(|s| s.to_ascii_lowercase())
    }

    pub fn filter_status(&self) -> Option<String> {
        Self::filter_text(&self.status)
    }

    pub fn filter_q(&self) -> Option<String> {
        Self::filter_text(&self.q).map(|s| s.to_ascii_lowercase())
    }

    pub fn filter_outcome(&self) -> Option<String> {
        Self::filter_text(&self.outcome)
    }

    pub fn filter_kind(&self) -> Option<String> {
        Self::filter_text(&self.kind)
    }

    pub fn filter_activity_type(&self) -> Option<String> {
        Self::filter_text(&self.activity_type).map(|s| s.to_ascii_lowercase())
    }

    pub fn parse_wallets(&self) -> Vec<Uuid> {
        let Some(raw) = self.wallets.as_ref() else {
            return Vec::new();
        };
        raw.split(',')
            .filter_map(|part| Uuid::parse_str(part.trim()).ok())
            .collect()
    }

    pub fn parse_from(&self) -> Option<DateTime<Utc>> {
        parse_ts(&self.from)
    }

    /// Exclusive upper bound for SQL (`column < bound`).
    /// Date-only `to` includes the entire UTC calendar day.
    pub fn parse_to_exclusive(&self) -> Option<DateTime<Utc>> {
        parse_to_exclusive(&self.to)
    }

    pub fn parse_decimal(value: &Option<String>) -> Option<Decimal> {
        value
            .as_ref()
            .and_then(|s| Decimal::from_str(s.trim()).ok())
    }

    pub fn min_amount(&self) -> Option<Decimal> {
        Self::parse_decimal(&self.min_amount)
    }

    pub fn max_amount(&self) -> Option<Decimal> {
        Self::parse_decimal(&self.max_amount)
    }

    pub fn min_price(&self) -> Option<Decimal> {
        Self::parse_decimal(&self.min_price)
    }

    pub fn max_price(&self) -> Option<Decimal> {
        Self::parse_decimal(&self.max_price)
    }
}

fn is_date_only(raw: &str) -> bool {
    raw.len() == 10 && NaiveDate::parse_from_str(raw, "%Y-%m-%d").is_ok()
}

fn parse_ts(value: &Option<String>) -> Option<DateTime<Utc>> {
    let raw = value.as_ref()?.trim();
    if raw.is_empty() {
        return None;
    }
    if is_date_only(raw) {
        let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok()?;
        let naive = date.and_hms_opt(0, 0, 0)?;
        return Some(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));
    }
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn parse_to_exclusive(value: &Option<String>) -> Option<DateTime<Utc>> {
    let raw = value.as_ref()?.trim();
    if raw.is_empty() {
        return None;
    }
    if is_date_only(raw) {
        let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d").ok()?;
        let next = date.succ_opt()?;
        let naive = next.and_hms_opt(0, 0, 0)?;
        return Some(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));
    }
    parse_ts(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_computes_offset() {
        let q = ListQuery {
            limit: Some(25),
            offset: None,
            page: Some(3),
            q: None,
            side: None,
            status: None,
            mode: None,
            account_id: None,
            wallets: None,
            from: None,
            to: None,
            min_amount: None,
            max_amount: None,
            min_price: None,
            max_price: None,
            outcome: None,
            kind: None,
            activity_type: None,
        };
        assert_eq!(q.offset(), 50);
        assert_eq!(q.limit(), 25);
    }

    #[test]
    fn blank_filters_are_absent() {
        let q = ListQuery {
            side: Some(String::new()),
            status: Some("  ".into()),
            ..ListQuery {
                limit: None,
                offset: None,
                page: None,
                q: None,
                side: None,
                status: None,
                mode: None,
                account_id: None,
                wallets: None,
                from: None,
                to: None,
                min_amount: None,
                max_amount: None,
                min_price: None,
                max_price: None,
                outcome: None,
                kind: None,
                activity_type: None,
            }
        };
        assert!(q.filter_side().is_none());
        assert!(q.filter_status().is_none());
    }

    #[test]
    fn date_from_inclusive_start() {
        let q = ListQuery {
            from: Some("2024-02-29".into()),
            ..empty_query()
        };
        let from = q.parse_from().unwrap();
        assert_eq!(from.to_rfc3339(), "2024-02-29T00:00:00+00:00");
    }

    #[test]
    fn date_to_exclusive_end_of_day_via_next_day() {
        let q = ListQuery {
            to: Some("2024-02-29".into()),
            ..empty_query()
        };
        let bound = q.parse_to_exclusive().unwrap();
        assert_eq!(bound.to_rfc3339(), "2024-03-01T00:00:00+00:00");
    }

    #[test]
    fn date_single_day_range_bounds() {
        let day = "2025-01-15";
        let q = ListQuery {
            from: Some(day.into()),
            to: Some(day.into()),
            ..empty_query()
        };
        assert_eq!(
            q.parse_from().unwrap().to_rfc3339(),
            "2025-01-15T00:00:00+00:00"
        );
        assert_eq!(
            q.parse_to_exclusive().unwrap().to_rfc3339(),
            "2025-01-16T00:00:00+00:00"
        );
    }

    #[test]
    fn date_month_end_roll() {
        let q = ListQuery {
            to: Some("2025-01-31".into()),
            ..empty_query()
        };
        assert_eq!(
            q.parse_to_exclusive().unwrap().to_rfc3339(),
            "2025-02-01T00:00:00+00:00"
        );
    }

    #[test]
    fn date_year_end_roll() {
        let q = ListQuery {
            to: Some("2025-12-31".into()),
            ..empty_query()
        };
        assert_eq!(
            q.parse_to_exclusive().unwrap().to_rfc3339(),
            "2026-01-01T00:00:00+00:00"
        );
    }

    #[test]
    fn rfc3339_to_uses_exact_exclusive_instant() {
        let q = ListQuery {
            to: Some("2025-06-10T14:30:00Z".into()),
            ..empty_query()
        };
        assert_eq!(
            q.parse_to_exclusive().unwrap().to_rfc3339(),
            "2025-06-10T14:30:00+00:00"
        );
    }

    #[test]
    fn events_through_end_of_utc_day_are_below_exclusive_bound() {
        use chrono::TimeZone;
        let q = ListQuery {
            to: Some("2025-01-15".into()),
            ..empty_query()
        };
        let bound = q.parse_to_exclusive().unwrap();
        let end_of_day = Utc.with_ymd_and_hms(2025, 1, 15, 23, 59, 59).unwrap();
        let next_day = Utc.with_ymd_and_hms(2025, 1, 16, 0, 0, 0).unwrap();
        assert!(end_of_day < bound);
        assert!(!(next_day < bound));
    }

    #[test]
    fn rfc3339_from_parsed_with_offset() {
        let q = ListQuery {
            from: Some("2025-06-10T14:30:00+02:00".into()),
            ..empty_query()
        };
        assert_eq!(
            q.parse_from().unwrap().to_rfc3339(),
            "2025-06-10T12:30:00+00:00"
        );
    }

    fn empty_query() -> ListQuery {
        ListQuery {
            limit: None,
            offset: None,
            page: None,
            q: None,
            side: None,
            status: None,
            mode: None,
            account_id: None,
            wallets: None,
            from: None,
            to: None,
            min_amount: None,
            max_amount: None,
            min_price: None,
            max_price: None,
            outcome: None,
            kind: None,
            activity_type: None,
        }
    }

    #[test]
    fn cap_page_size_at_100() {
        let q = ListQuery {
            limit: Some(500),
            offset: None,
            page: None,
            q: None,
            side: None,
            status: None,
            mode: None,
            account_id: None,
            wallets: None,
            from: None,
            to: None,
            min_amount: None,
            max_amount: None,
            min_price: None,
            max_price: None,
            outcome: None,
            kind: None,
            activity_type: None,
        };
        assert_eq!(q.limit(), 100);
    }
}
