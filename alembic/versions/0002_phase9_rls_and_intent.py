"""Phase 9: Dedicated non-superuser LOGIN role `pommarket_app` (`NOSUPERUSER NOBYPASSRLS`) and durable order intent columns.

Revision ID: 0002_phase9_rls_and_intent
Revises: 0001_initial_schema
Create Date: 2026-11-08 16:05:00

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa

revision: str = "0002_phase9_rls_and_intent"
down_revision: Union[str, None] = "0001_initial_schema"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # 1. Durable order intent columns for crash-safe execution (Finding E-03)
    op.add_column("orders", sa.Column("client_order_id", sa.String(128), nullable=True))
    op.add_column("orders", sa.Column("eip712_order_hash", sa.String(128), nullable=True))
    op.add_column("orders", sa.Column("signed_payload_json", sa.JSON(), nullable=True))
    op.create_index("ix_orders_client_order_id", "orders", ["client_order_id"], unique=True)
    op.create_index("ix_orders_eip712_order_hash", "orders", ["eip712_order_hash"])

    # 2. Dedicated non-superuser LOGIN runtime role `pommarket_app` (Finding S-01)
    bind = op.get_bind()
    if bind.dialect.name == "postgresql":
        op.execute(
            """
            DO $$
            BEGIN
                IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'pommarket_app') THEN
                    CREATE ROLE pommarket_app LOGIN PASSWORD 'pommarket_app_pass'
                        NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
                ELSE
                    ALTER ROLE pommarket_app WITH LOGIN PASSWORD 'pommarket_app_pass'
                        NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
                END IF;
            END
            $$;
            GRANT USAGE ON SCHEMA public TO pommarket_app;
            GRANT SELECT, INSERT, UPDATE, DELETE, TRUNCATE ON ALL TABLES IN SCHEMA public TO pommarket_app;
            GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO pommarket_app;
            ALTER DEFAULT PRIVILEGES IN SCHEMA public
                GRANT SELECT, INSERT, UPDATE, DELETE, TRUNCATE ON TABLES TO pommarket_app;
            ALTER DEFAULT PRIVILEGES IN SCHEMA public
                GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO pommarket_app;
            """
        )


def downgrade() -> None:
    op.drop_index("ix_orders_eip712_order_hash", table_name="orders")
    op.drop_index("ix_orders_client_order_id", table_name="orders")
    op.drop_column("orders", "signed_payload_json")
    op.drop_column("orders", "eip712_order_hash")
    op.drop_column("orders", "client_order_id")
