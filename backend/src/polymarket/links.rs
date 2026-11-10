fn trim_slug(slug: Option<&str>) -> Option<&str> {
    slug.map(str::trim).filter(|s| !s.is_empty())
}

/// Build a Polymarket URL from activity slugs.
/// Data API `slug` maps to `/market/{slug}`; parent `eventSlug` maps to `/event/{slug}`.
pub fn polymarket_event_url(event_slug: Option<&str>, market_slug: Option<&str>) -> Option<String> {
    let event = trim_slug(event_slug);
    let market = trim_slug(market_slug);

    if let Some(m) = market {
        if event.is_none() || event != Some(m) {
            return Some(format!("https://polymarket.com/market/{m}"));
        }
    }
    event
        .map(|slug| format!("https://polymarket.com/event/{slug}"))
        .or_else(|| market.map(|slug| format!("https://polymarket.com/market/{slug}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_slug_uses_market_path_when_event_differs() {
        let url = polymarket_event_url(
            Some("ethereum-above-on-november-10-2026"),
            Some("ethereum-above-2200-on-november-10-2026"),
        )
        .unwrap();
        assert_eq!(
            url,
            "https://polymarket.com/market/ethereum-above-2200-on-november-10-2026"
        );
    }

    #[test]
    fn event_only_uses_event_path() {
        let url = polymarket_event_url(Some("parent-event"), None).unwrap();
        assert_eq!(url, "https://polymarket.com/event/parent-event");
    }

    #[test]
    fn market_only_uses_market_path() {
        let url = polymarket_event_url(None, Some("solo-market")).unwrap();
        assert_eq!(url, "https://polymarket.com/market/solo-market");
    }

    #[test]
    fn identical_slugs_use_event_path() {
        let url = polymarket_event_url(Some("same"), Some("same")).unwrap();
        assert_eq!(url, "https://polymarket.com/event/same");
    }
}
