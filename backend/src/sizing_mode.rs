//! Copy strategy sizing mode names and API validation.

/// Modes exposed in UI and allowed on create.
pub const USER_SIZING_MODES: &[&str] = &["position_value_ratio", "fixed_amount", "proportional"];

/// Deprecated modes still honored by the pipeline for existing strategies.
pub const LEGACY_SIZING_MODES: &[&str] = &["portfolio_ratio"];

/// Removed from UI after 0019; parse_mode still accepts `multiplier` if any row predates migration.
pub const RETIRED_SIZING_MODES: &[&str] = &["multiplier"];

pub fn is_valid_sizing_mode(mode: &str) -> bool {
    USER_SIZING_MODES.contains(&mode)
        || LEGACY_SIZING_MODES.contains(&mode)
        || RETIRED_SIZING_MODES.contains(&mode)
}

pub fn is_user_sizing_mode(mode: &str) -> bool {
    USER_SIZING_MODES.contains(&mode)
}

/// Modes where Small BUY Override can be stored in API/DB (sizing applies it for all stored modes).
pub fn supports_small_buy_override_storage(mode: &str) -> bool {
    mode == "position_value_ratio" || mode == "proportional"
}

pub fn reject_retired_on_write(mode: &str) -> Option<&'static str> {
    if RETIRED_SIZING_MODES.contains(&mode) {
        Some("multiplier sizing was merged into proportional; set sizing_mode to proportional and use copy_percent or proportion")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_modes_list() {
        assert_eq!(USER_SIZING_MODES.len(), 3);
    }

    #[test]
    fn multiplier_retired_multiplier_valid_for_read() {
        assert!(is_valid_sizing_mode("multiplier"));
        assert!(!is_user_sizing_mode("multiplier"));
        assert!(reject_retired_on_write("multiplier").is_some());
    }

    #[test]
    fn small_buy_storage_modes() {
        assert!(supports_small_buy_override_storage("position_value_ratio"));
        assert!(supports_small_buy_override_storage("proportional"));
        assert!(!supports_small_buy_override_storage("fixed_amount"));
    }
}
