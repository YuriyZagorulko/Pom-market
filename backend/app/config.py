"""Platform configuration loaded from environment variables."""

from __future__ import annotations

import os
from dataclasses import dataclass, field


@dataclass(frozen=True)
class Settings:
    app_name: str = "Pom-Market Multi-Tenant Copy-Trading Platform"
    environment: str = field(default_factory=lambda: os.getenv("ENVIRONMENT", "development"))

    # Dedicated non-default development host ports
    frontend_port: int = field(default_factory=lambda: int(os.getenv("FRONTEND_PORT", "4319")))
    backend_port: int = field(default_factory=lambda: int(os.getenv("BACKEND_PORT", "4320")))
    rust_engine_port: int = field(default_factory=lambda: int(os.getenv("RUST_ENGINE_PORT", "4321")))
    postgres_port: int = field(default_factory=lambda: int(os.getenv("POSTGRES_PORT", "5440")))
    redis_port: int = field(default_factory=lambda: int(os.getenv("REDIS_PORT", "6390")))
    nats_port: int = field(default_factory=lambda: int(os.getenv("NATS_PORT", "4225")))
    nats_monitor_port: int = field(default_factory=lambda: int(os.getenv("NATS_MONITOR_PORT", "8225")))
    prometheus_port: int = field(default_factory=lambda: int(os.getenv("PROMETHEUS_PORT", "9095")))
    grafana_port: int = field(default_factory=lambda: int(os.getenv("GRAFANA_PORT", "3015")))

    database_url: str = field(
        default_factory=lambda: os.getenv(
            "DATABASE_URL",
            f"postgresql+psycopg2://pommarket_app:pommarket_app_pass@localhost:{os.getenv('POSTGRES_PORT', '5440')}/pommarket",
        )
    )
    migration_database_url: str = field(
        default_factory=lambda: os.getenv(
            "MIGRATION_DATABASE_URL",
            f"postgresql+psycopg2://pommarket:pommarket_dev_pass@localhost:{os.getenv('POSTGRES_PORT', '5440')}/pommarket",
        )
    )
    redis_url: str = field(
        default_factory=lambda: os.getenv(
            "REDIS_URL",
            f"redis://localhost:{os.getenv('REDIS_PORT', '6390')}/0",
        )
    )
    nats_url: str = field(
        default_factory=lambda: os.getenv(
            "NATS_URL",
            f"nats://localhost:{os.getenv('NATS_PORT', '4225')}",
        )
    )
    nats_monitor_url: str = field(
        default_factory=lambda: os.getenv(
            "NATS_MONITOR_URL",
            f"http://localhost:{os.getenv('NATS_MONITOR_PORT', '8225')}",
        )
    )
    rust_engine_url: str = field(
        default_factory=lambda: os.getenv(
            "RUST_ENGINE_INTERNAL_URL",
            f"http://localhost:{os.getenv('RUST_ENGINE_PORT', '4321')}",
        )
    )
    cors_allowed_origins: list[str] = field(
        default_factory=lambda: [
            origin.strip()
            for origin in os.getenv(
                "CORS_ALLOWED_ORIGINS",
                f"http://localhost:{os.getenv('FRONTEND_PORT', '4319')},http://127.0.0.1:{os.getenv('FRONTEND_PORT', '4319')}",
            ).split(",")
            if origin.strip()
        ]
    )

    jwt_secret_key: str = field(
        default_factory=lambda: os.getenv(
            "JWT_SECRET_KEY",
            "dev-only-jwt-secret-key-replace-in-production-32bytes!",
        )
    )
    jwt_algorithm: str = "HS256"
    access_token_expire_minutes: int = 30
    refresh_token_expire_days: int = 7

    # Master key for AES-256-GCM authenticated encryption of Polymarket signing secrets.
    # Must be kept outside the database (supplied via environment or secret manager).
    master_encryption_key: str = field(
        default_factory=lambda: os.getenv(
            "MASTER_ENCRYPTION_KEY",
            "dev-only-master-encryption-key-32-bytes-minimum-seed!",
        )
    )
    master_encryption_key_previous: str = field(
        default_factory=lambda: os.getenv("MASTER_ENCRYPTION_KEY_PREVIOUS", "")
    )

    # Polymarket official endpoints
    clob_host: str = field(
        default_factory=lambda: os.getenv("POLYMARKET_CLOB_HOST", "https://clob.polymarket.com")
    )
    gamma_host: str = field(
        default_factory=lambda: os.getenv("POLYMARKET_GAMMA_HOST", "https://gamma-api.polymarket.com")
    )
    data_api_host: str = field(
        default_factory=lambda: os.getenv("POLYMARKET_DATA_API_HOST", "https://data-api.polymarket.com")
    )
    rtds_ws_url: str = field(
        default_factory=lambda: os.getenv("POLYMARKET_RTDS_WS_URL", "wss://ws-live-data.polymarket.com")
    )
    polygon_chain_id: int = field(
        default_factory=lambda: int(os.getenv("POLYGON_CHAIN_ID", "137"))
    )
    ctf_exchange_v2_address: str = field(
        default_factory=lambda: os.getenv(
            "POLYGON_CTF_EXCHANGE_V2_ADDRESS",
            "0xE111180000d2663C0091e4f400237545B87B996B",
        )
    )
    neg_risk_ctf_exchange_v2_address: str = field(
        default_factory=lambda: os.getenv(
            "POLYGON_NEG_RISK_CTF_EXCHANGE_V2_ADDRESS",
            "0xe2222d279d744050d28e00520010520000310F59",
        )
    )

    # Platform safety defaults
    default_operating_mode: str = "MONITORING"
    global_rate_limit_rps: float = field(
        default_factory=lambda: float(os.getenv("GLOBAL_RATE_LIMIT_RPS", "15.0"))
    )
    per_account_rate_limit_rpm: int = field(
        default_factory=lambda: int(os.getenv("PER_ACCOUNT_RATE_LIMIT_RPM", "30"))
    )
    stale_data_threshold_seconds: int = field(
        default_factory=lambda: int(os.getenv("STALE_DATA_THRESHOLD_SECONDS", "60"))
    )
    email_delivery_enabled: bool = field(
        default_factory=lambda: os.getenv("EMAIL_DELIVERY_ENABLED", "false").lower() == "true"
    )


def get_settings() -> Settings:
    return Settings()
