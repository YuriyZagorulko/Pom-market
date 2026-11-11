//! API sizing guard logic (no HTTP; complements integration tests).

use pommarket::sizing_mode::{is_user_sizing_mode, is_valid_sizing_mode, reject_retired_on_write, USER_SIZING_MODES};

#[test]
fn only_three_user_modes() {
    assert_eq!(USER_SIZING_MODES.len(), 3);
    for m in USER_SIZING_MODES {
        assert!(is_user_sizing_mode(m));
    }
    assert!(!is_user_sizing_mode("portfolio_ratio"));
    assert!(is_valid_sizing_mode("portfolio_ratio"));
}

#[test]
fn cannot_adopt_retired_multiplier_via_api_validation() {
    assert!(reject_retired_on_write("multiplier").is_some());
}
