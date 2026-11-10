//! Per-account trading control state (danger zone).

use serde::{Deserialize, Serialize};

pub const STATE_NORMAL: &str = "normal";
pub const STATE_KILL_SWITCH: &str = "kill_switch";
pub const STATE_SOFT_CLOSE: &str = "soft_close";
pub const STATE_HARD_CLOSE_IN_PROGRESS: &str = "hard_close_in_progress";
pub const STATE_HARD_CLOSE_COMPLETED: &str = "hard_close_completed";
pub const STATE_HARD_CLOSE_PARTIAL: &str = "hard_close_partial";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradingControlState {
    Normal,
    KillSwitch,
    SoftClose,
    HardCloseInProgress,
    HardCloseCompleted,
    HardClosePartial,
}

impl TradingControlState {
    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            STATE_NORMAL => Self::Normal,
            STATE_KILL_SWITCH => Self::KillSwitch,
            STATE_SOFT_CLOSE => Self::SoftClose,
            STATE_HARD_CLOSE_IN_PROGRESS => Self::HardCloseInProgress,
            STATE_HARD_CLOSE_COMPLETED => Self::HardCloseCompleted,
            STATE_HARD_CLOSE_PARTIAL => Self::HardClosePartial,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => STATE_NORMAL,
            Self::KillSwitch => STATE_KILL_SWITCH,
            Self::SoftClose => STATE_SOFT_CLOSE,
            Self::HardCloseInProgress => STATE_HARD_CLOSE_IN_PROGRESS,
            Self::HardCloseCompleted => STATE_HARD_CLOSE_COMPLETED,
            Self::HardClosePartial => STATE_HARD_CLOSE_PARTIAL,
        }
    }

    pub fn blocks_all_trading(self) -> bool {
        matches!(self, Self::KillSwitch | Self::HardCloseInProgress)
    }

    pub fn blocks_buy(self) -> bool {
        matches!(
            self,
            Self::KillSwitch
                | Self::SoftClose
                | Self::HardCloseInProgress
                | Self::HardCloseCompleted
                | Self::HardClosePartial
        )
    }

    pub fn allows_copy_sell(self) -> bool {
        matches!(
            self,
            Self::Normal | Self::SoftClose | Self::HardClosePartial | Self::HardCloseCompleted
        )
    }

    pub fn emergency_stop_flag(self) -> bool {
        matches!(
            self,
            Self::KillSwitch
                | Self::HardCloseInProgress
                | Self::HardCloseCompleted
                | Self::HardClosePartial
        )
    }
}

pub fn pipeline_blocked(
    state: &str,
    side_buy: bool,
    global_kill: bool,
    operating_mode: &str,
) -> Option<&'static str> {
    if global_kill && operating_mode != "monitoring" {
        return Some("kill_switch");
    }
    let Some(parsed) = TradingControlState::parse(state) else {
        return Some("account_trading_halted");
    };
    if parsed == TradingControlState::KillSwitch {
        return Some("emergency_stop");
    }
    if parsed == TradingControlState::HardCloseInProgress {
        return Some("hard_close_in_progress");
    }
    if side_buy && parsed.blocks_buy() {
        return Some(if parsed == TradingControlState::SoftClose {
            "soft_close_no_buy"
        } else {
            "account_no_buy"
        });
    }
    if !side_buy && !parsed.allows_copy_sell() {
        return Some("account_trading_halted");
    }
    None
}

pub fn can_start_hard_close(from: TradingControlState) -> bool {
    matches!(
        from,
        TradingControlState::Normal
            | TradingControlState::SoftClose
            | TradingControlState::HardCloseCompleted
            | TradingControlState::HardClosePartial
    )
}

pub fn resume_requires_confirmation(from: TradingControlState) -> bool {
    matches!(
        from,
        TradingControlState::KillSwitch
            | TradingControlState::SoftClose
            | TradingControlState::HardCloseCompleted
            | TradingControlState::HardClosePartial
    )
}

pub fn next_state_after_hard_close(all_closed: bool, _any_progress: bool) -> &'static str {
    if all_closed {
        STATE_HARD_CLOSE_COMPLETED
    } else {
        STATE_HARD_CLOSE_PARTIAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_blocks_both_sides() {
        assert_eq!(
            pipeline_blocked(STATE_KILL_SWITCH, true, false, "paper_trading"),
            Some("emergency_stop")
        );
        assert_eq!(
            pipeline_blocked(STATE_KILL_SWITCH, false, false, "paper_trading"),
            Some("emergency_stop")
        );
    }

    #[test]
    fn soft_close_blocks_buy_allows_sell() {
        assert_eq!(
            pipeline_blocked(STATE_SOFT_CLOSE, true, false, "paper_trading"),
            Some("soft_close_no_buy")
        );
        assert!(pipeline_blocked(STATE_SOFT_CLOSE, false, false, "paper_trading").is_none());
    }

    #[test]
    fn global_kill_blocks_paper() {
        assert_eq!(
            pipeline_blocked(STATE_NORMAL, true, true, "paper_trading"),
            Some("kill_switch")
        );
    }

    #[test]
    fn accounts_do_not_share_state() {
        let a = pipeline_blocked(STATE_KILL_SWITCH, true, false, "paper_trading");
        let b = pipeline_blocked(STATE_NORMAL, true, false, "paper_trading");
        assert!(a.is_some());
        assert!(b.is_none());
    }
}
