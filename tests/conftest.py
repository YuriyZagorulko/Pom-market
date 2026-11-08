"""Pytest fixtures for PostgreSQL multi-tenant test suite using the non-superuser runtime role `pommarket_app`."""

from __future__ import annotations

import os
import pytest
from sqlalchemy import text

POSTGRES_PORT = os.getenv("POSTGRES_PORT", "5440")
os.environ.setdefault(
    "DATABASE_URL",
    f"postgresql+psycopg2://pommarket_app:pommarket_app_pass@localhost:{POSTGRES_PORT}/pommarket_test",
)
os.environ.setdefault(
    "MIGRATION_DATABASE_URL",
    f"postgresql+psycopg2://pommarket:pommarket_dev_pass@localhost:{POSTGRES_PORT}/pommarket_test",
)

from backend.app.db import get_engine, get_session_factory, set_tenant_context
from backend.app.services.email_service import DEV_MAILBOX


@pytest.fixture()
def db_session():
    """Provide a clean PostgreSQL database session for each test using `pommarket_app`."""
    DEV_MAILBOX.clear()
    engine = get_engine()
    with engine.begin() as conn:
        conn.execute(
            text(
                """
                TRUNCATE TABLE
                    system_settings,
                    account_activity_logs,
                    account_balances,
                    virtual_positions,
                    fills,
                    orders,
                    execution_jobs,
                    source_trade_events,
                    risk_limits,
                    copy_strategies,
                    account_source_traders,
                    source_traders,
                    trading_account_credentials,
                    trading_accounts,
                    user_sessions,
                    user_roles,
                    users
                CASCADE;
                """
            )
        )
    factory = get_session_factory()
    session = factory()
    try:
        set_tenant_context(session, user_id=None, is_super_admin=True)
        yield session
        session.commit()
    finally:
        session.info.clear()
        session.close()
