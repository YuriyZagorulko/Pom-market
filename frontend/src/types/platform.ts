export type OperatingMode = "MONITORING" | "PAPER_TRADING" | "LIVE_TRADING";
export type ExecutionMode =
  | "FIXED_AMOUNT"
  | "FIXED_PERCENTAGE"
  | "PROPORTIONAL_BALANCE"
  | "MAX_ORDER_SIZE";
export type SignatureType = "EOA" | "POLY_PROXY" | "POLY_GNOSIS_SAFE" | "POLY_1271";

export interface AccountPerformanceRow {
  account_id: number;
  display_name: string;
  operating_mode: OperatingMode;
  connection_status: string;
  trading_status: string;
  emergency_stop: boolean;
  available_cash_usd: string;
  current_exposure_usd: string;
  total_equity_usd: string;
  daily_pnl_usd: string;
  weekly_pnl_usd: string;
  monthly_pnl_usd: string;
}

export interface DashboardSummary {
  total_trading_accounts: number;
  connected_accounts: number;
  disconnected_accounts: number;
  active_copy_trading_accounts: number;
  monitored_source_traders: number;
  total_copied_trades: number;
  daily_pnl_usd: string;
  weekly_pnl_usd: string;
  monthly_pnl_usd: string;
  current_exposure_usd: string;
  account_performance: AccountPerformanceRow[];
  recent_orders: Array<{
    id: number;
    trading_account_id: number;
    operating_mode: OperatingMode;
    token_id: string;
    side: "BUY" | "SELL";
    limit_price: string;
    requested_size_shares: string;
    filled_size_shares: string;
    status: string;
    error_message: string | null;
    order_submitted_ts: string;
  }>;
  recent_fills: Array<{
    id: number;
    order_id: number;
    trading_account_id: number;
    operating_mode: OperatingMode;
    token_id: string;
    side: "BUY" | "SELL";
    filled_shares: string;
    avg_fill_price: string;
    notional_usd: string;
    fee_usd: string;
    realized_pnl_usd: string;
    fill_ts: string;
  }>;
  execution_errors: Array<{
    job_id: number;
    trading_account_id: number;
    operating_mode: OperatingMode;
    status: string;
    reason: string | null;
    created_at: string;
  }>;
}

export interface TradingAccountDetail {
  account: {
    id: number;
    user_id: number;
    display_name: string;
    funder_wallet_address: string | null;
    signer_wallet_address: string;
    signature_type: SignatureType;
    operating_mode: OperatingMode;
    connection_status: string;
    authorization_status: string;
    trading_status: string;
    emergency_stop: boolean;
    emergency_stop_reason: string | null;
    consecutive_errors: number;
    circuit_breaker_tripped: boolean;
    has_signing_key: boolean;
    has_api_credentials: boolean;
  };
  balances: Array<{
    operating_mode: OperatingMode;
    available_cash_usd: string;
    locked_cash_usd: string;
    positions_market_value_usd: string;
    total_equity_usd: string;
    daily_realized_pnl_usd: string;
    weekly_realized_pnl_usd: string;
    monthly_realized_pnl_usd: string;
    all_time_realized_pnl_usd: string;
  }>;
  positions: Array<{
    id: number;
    source_wallet: string;
    market_condition_id: string;
    token_id: string;
    operating_mode: OperatingMode;
    open_shares: string;
    avg_entry_price: string;
    cost_basis_usd: string;
    realized_pnl_usd: string;
    total_fees_usd: string;
    source_observed_shares: string;
    pending_exit_shares: string;
  }>;
  orders: DashboardSummary["recent_orders"];
  trade_history: DashboardSummary["recent_fills"];
  source_traders: Array<{
    source_trader_id: number;
    wallet_address: string;
    custom_name: string | null;
    is_enabled: boolean;
    execution_mode: ExecutionMode;
    fixed_amount_usd: string;
    copy_percentage: string;
    copy_multiplier: string;
    max_order_size_usd: string;
    max_slippage_pct: string;
  }>;
  copy_configuration: {
    execution_mode: ExecutionMode;
    fixed_amount_usd: string;
    source_size_percentage: string;
    copy_multiplier: string;
    max_order_size_usd: string;
    min_order_size_usd: string;
    max_slippage_pct: string;
    buy_enabled: boolean;
    sell_enabled: boolean;
    slice_collapse_window_secs: number;
    allowed_markets: string[] | null;
  } | null;
  risk_limits: {
    max_order_size_usd: string;
    min_order_size_usd: string;
    max_market_exposure_usd: string;
    max_source_trader_exposure_usd: string;
    max_total_exposure_usd: string;
    max_daily_loss_usd: string;
    max_slippage_pct: string;
    max_executions_per_interval: number;
    execution_interval_seconds: number;
    daily_loss_stop_latched: boolean;
  } | null;
}

export interface SourceTraderPageDetail {
  source_trader_id: number;
  wallet_address: string;
  default_label: string | null;
  total_observed_trades: number;
  total_observed_volume_usd: string;
  estimated_balance_usd: string;
  win_rate_pct: string | null;
  realized_roi_pct: string | null;
  markets: string[];
  recent_trades: Array<{
    id: number;
    tx_hash: string;
    market_condition_id: string;
    market_question: string | null;
    outcome_label: string | null;
    token_id: string;
    side: "BUY" | "SELL";
    price: string;
    size: string;
    notional_usd: string;
    source_event_ts: string;
  }>;
  copying_destination_accounts: Array<{
    trading_account_id: number;
    display_name: string;
    operating_mode: OperatingMode;
    custom_name: string | null;
    is_enabled: boolean;
    execution_mode: ExecutionMode;
    fixed_amount_usd: string;
    copy_percentage: string;
    copy_multiplier: string;
    max_order_size_usd: string;
    max_slippage_pct: string;
  }>;
}

export interface AdminOverview {
  platform_health: string;
  global_emergency_stop: boolean;
  services: Record<string, string>;
  aggregates: {
    registered_users: number;
    suspended_users: number;
    trading_accounts: number;
    monitored_source_traders: number;
    account_source_mappings: number;
    source_events_processed: number;
    queued_execution_jobs: number;
    ambiguous_orders_in_flight: number;
    failed_execution_jobs: number;
  };
  resource_usage: {
    pid: number;
    max_rss_mb: number;
    user_cpu_seconds: number;
    system_cpu_seconds: number;
  };
  registered_users: Array<{
    id: number;
    email: string;
    full_name: string | null;
    is_active: boolean;
    is_suspended: boolean;
    email_verified: boolean;
    totp_enabled: boolean;
    roles: string[];
    created_at: string;
  }>;
  recent_errors: Array<{
    job_id: number;
    user_id: number;
    trading_account_id: number;
    status: string;
    reason: string | null;
    created_at: string;
  }>;
}
