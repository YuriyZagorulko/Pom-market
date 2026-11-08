"""Polymarket trading account management, tenant ownership enforcement, and credential vaulting."""

from __future__ import annotations

from datetime import datetime, timezone
from decimal import Decimal
from typing import Optional

from sqlalchemy import select
from sqlalchemy.orm import Session

from backend.app.models import (
    AccountActivityLog,
    AccountBalance,
    CopyStrategy,
    ExecutionMode,
    OperatingMode,
    RiskLimit,
    SignatureType,
    TradingAccount,
    TradingAccountCredential,
)
from backend.app.schemas import (
    CopyStrategyResponse,
    CopyStrategyUpdateRequest,
    CredentialRotateRequest,
    EmergencyStopRequest,
    OperatingModeChangeRequest,
    RiskLimitResponse,
    RiskLimitUpdateRequest,
    TradingAccountCreateRequest,
    TradingAccountResponse,
)
from backend.app.security import CredentialVault


class TenantAccessDeniedError(PermissionError):
    """Raised when a user attempts to access or modify another tenant's resource."""


class AccountValidationError(ValueError):
    """Raised when trading account parameters or mode transitions are invalid."""


def _to_account_response(account: TradingAccount) -> TradingAccountResponse:
    cred = account.credentials
    has_key = bool(cred and cred.encrypted_private_key and cred.is_connected)
    has_api = bool(cred and cred.encrypted_api_key and cred.encrypted_api_secret and cred.is_connected)
    return TradingAccountResponse(
        id=account.id,
        user_id=account.user_id,
        display_name=account.display_name,
        funder_wallet_address=account.funder_wallet_address,
        signer_wallet_address=account.signer_wallet_address,
        signature_type=account.signature_type,
        operating_mode=account.operating_mode,
        connection_status=account.connection_status,
        authorization_status=account.authorization_status,
        trading_status=account.trading_status,
        emergency_stop=account.emergency_stop,
        emergency_stop_reason=account.emergency_stop_reason,
        consecutive_errors=account.consecutive_errors,
        circuit_breaker_tripped=account.circuit_breaker_tripped,
        has_signing_key=has_key,
        has_api_credentials=has_api,
        key_version=cred.key_version if cred else None,
        created_at=account.created_at,
        updated_at=account.updated_at,
    )


class TradingAccountService:
    def __init__(self, vault: Optional[CredentialVault] = None) -> None:
        self._vault = vault or CredentialVault()

    def get_owned_account(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
    ) -> TradingAccount:
        """Centralized tenant ownership guard: enforces that `account.user_id == user_id`."""
        account = session.scalar(
            select(TradingAccount).where(
                TradingAccount.id == account_id,
                TradingAccount.user_id == user_id,
            )
        )
        if account is None:
            raise TenantAccessDeniedError(
                f"Trading account {account_id} does not exist or does not belong to user {user_id}"
            )
        return account

    def create_trading_account(
        self,
        session: Session,
        *,
        user_id: int,
        req: TradingAccountCreateRequest,
    ) -> TradingAccountResponse:
        # Validate Polymarket wallet & signature model rules:
        # - For EOA: funder address is identical to signer_wallet_address
        # - For POLY_PROXY / POLY_GNOSIS_SAFE / POLY_1271: funder_wallet_address (the proxy/Safe) must be provided
        if req.signature_type == SignatureType.EOA:
            funder_address = req.funder_wallet_address or req.signer_wallet_address
            if funder_address != req.signer_wallet_address:
                raise AccountValidationError(
                    "For EOA signature type, funder_wallet_address must match signer_wallet_address"
                )
        else:
            if not req.funder_wallet_address:
                raise AccountValidationError(
                    f"Signature type {req.signature_type.value} requires an explicit funder_wallet_address (proxy/Safe address)"
                )
            funder_address = req.funder_wallet_address

        # Default mode is strictly MONITORING per safety requirements
        account = TradingAccount(
            user_id=user_id,
            display_name=req.display_name.strip(),
            funder_wallet_address=funder_address,
            signer_wallet_address=req.signer_wallet_address,
            signature_type=req.signature_type.value,
            operating_mode=OperatingMode.MONITORING.value,
            connection_status="CONNECTED",
            authorization_status="AUTHORIZED",
            trading_status="ACTIVE",
            emergency_stop=False,
        )
        session.add(account)
        session.flush()

        # Encrypt sensitive credentials at rest with tenant+account AAD binding
        cred = TradingAccountCredential(
            user_id=user_id,
            trading_account_id=account.id,
            encrypted_private_key=(
                self._vault.encrypt_secret(req.private_key, user_id=user_id, trading_account_id=account.id)
                if req.private_key
                else None
            ),
            encrypted_api_key=(
                self._vault.encrypt_secret(req.api_key, user_id=user_id, trading_account_id=account.id)
                if req.api_key
                else None
            ),
            encrypted_api_secret=(
                self._vault.encrypt_secret(req.api_secret, user_id=user_id, trading_account_id=account.id)
                if req.api_secret
                else None
            ),
            encrypted_api_passphrase=(
                self._vault.encrypt_secret(req.api_passphrase, user_id=user_id, trading_account_id=account.id)
                if req.api_passphrase
                else None
            ),
            key_version=1,
            is_connected=True,
        )
        session.add(cred)

        # Create default independent risk limits for this account
        risk = RiskLimit(
            user_id=user_id,
            trading_account_id=account.id,
            max_order_size_usd=Decimal("100.0"),
            min_order_size_usd=Decimal("1.0"),
            max_market_exposure_usd=Decimal("500.0"),
            max_source_trader_exposure_usd=Decimal("500.0"),
            max_total_exposure_usd=Decimal("2500.0"),
            max_daily_loss_usd=Decimal("200.0"),
            max_slippage_pct=Decimal("0.05"),
            max_executions_per_interval=10,
            execution_interval_seconds=60,
            min_market_liquidity_usd=Decimal("5.0"),
            max_stale_data_seconds=60,
            circuit_breaker_error_threshold=5,
        )
        session.add(risk)

        # Create default copy strategy for this account
        strategy = CopyStrategy(
            user_id=user_id,
            trading_account_id=account.id,
            name="Default Strategy",
            is_active=True,
            execution_mode=ExecutionMode.FIXED_AMOUNT.value,
            fixed_amount_usd=Decimal("10.0"),
            source_size_percentage=Decimal("0.10"),
            copy_multiplier=Decimal("1.0"),
            max_order_size_usd=Decimal("100.0"),
            min_order_size_usd=Decimal("1.0"),
            max_slippage_pct=Decimal("0.05"),
            buy_enabled=True,
            sell_enabled=True,
        )
        session.add(strategy)

        # Initialize independent account balances for PAPER_TRADING and LIVE_TRADING
        for mode in (OperatingMode.MONITORING.value, OperatingMode.PAPER_TRADING.value, OperatingMode.LIVE_TRADING.value):
            initial_cash = req.initial_paper_balance_usd if mode != OperatingMode.LIVE_TRADING.value else Decimal("0.0")
            session.add(
                AccountBalance(
                    user_id=user_id,
                    trading_account_id=account.id,
                    operating_mode=mode,
                    available_cash_usd=initial_cash,
                    locked_cash_usd=Decimal("0.0"),
                    positions_market_value_usd=Decimal("0.0"),
                    total_equity_usd=initial_cash,
                )
            )

        session.add(
            AccountActivityLog(
                user_id=user_id,
                trading_account_id=account.id,
                action="TRADING_ACCOUNT_CREATED",
                details_json={
                    "display_name": account.display_name,
                    "signature_type": account.signature_type,
                    "operating_mode": account.operating_mode,
                },
            )
        )
        session.flush()
        session.refresh(account)
        return _to_account_response(account)

    def list_user_accounts(self, session: Session, *, user_id: int) -> list[TradingAccountResponse]:
        accounts = session.scalars(
            select(TradingAccount)
            .where(TradingAccount.user_id == user_id)
            .order_by(TradingAccount.id.asc())
        ).all()
        return [_to_account_response(a) for a in accounts]

    def get_account_response(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
    ) -> TradingAccountResponse:
        account = self.get_owned_account(session, user_id=user_id, account_id=account_id)
        return _to_account_response(account)

    def rotate_credentials(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
        req: CredentialRotateRequest,
    ) -> TradingAccountResponse:
        account = self.get_owned_account(session, user_id=user_id, account_id=account_id)
        cred = account.credentials
        if cred is None:
            cred = TradingAccountCredential(
                user_id=user_id,
                trading_account_id=account.id,
                key_version=1,
                is_connected=True,
            )
            session.add(cred)

        next_version = cred.key_version + 1
        if req.private_key is not None:
            cred.encrypted_private_key = self._vault.encrypt_secret(
                req.private_key,
                user_id=user_id,
                trading_account_id=account.id,
                key_version=next_version,
            )
        if req.api_key is not None:
            cred.encrypted_api_key = self._vault.encrypt_secret(
                req.api_key,
                user_id=user_id,
                trading_account_id=account.id,
                key_version=next_version,
            )
        if req.api_secret is not None:
            cred.encrypted_api_secret = self._vault.encrypt_secret(
                req.api_secret,
                user_id=user_id,
                trading_account_id=account.id,
                key_version=next_version,
            )
        if req.api_passphrase is not None:
            cred.encrypted_api_passphrase = self._vault.encrypt_secret(
                req.api_passphrase,
                user_id=user_id,
                trading_account_id=account.id,
                key_version=next_version,
            )

        cred.key_version = next_version
        cred.is_connected = True
        cred.rotated_at = datetime.now(timezone.utc)
        account.connection_status = "CONNECTED"
        account.authorization_status = "AUTHORIZED"

        session.add(
            AccountActivityLog(
                user_id=user_id,
                trading_account_id=account.id,
                action="CREDENTIALS_ROTATED",
                details_json={"key_version": next_version},
            )
        )
        session.flush()
        session.refresh(account)
        return _to_account_response(account)

    def disconnect_account(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
    ) -> TradingAccountResponse:
        account = self.get_owned_account(session, user_id=user_id, account_id=account_id)
        if account.credentials is not None:
            account.credentials.encrypted_private_key = None
            account.credentials.encrypted_api_key = None
            account.credentials.encrypted_api_secret = None
            account.credentials.encrypted_api_passphrase = None
            account.credentials.is_connected = False

        account.connection_status = "DISCONNECTED"
        account.authorization_status = "REVOKED"
        account.trading_status = "PAUSED"
        account.operating_mode = OperatingMode.MONITORING.value

        session.add(
            AccountActivityLog(
                user_id=user_id,
                trading_account_id=account.id,
                action="ACCOUNT_DISCONNECTED",
                details_json={},
            )
        )
        session.flush()
        session.refresh(account)
        return _to_account_response(account)

    def change_operating_mode(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
        req: OperatingModeChangeRequest,
    ) -> TradingAccountResponse:
        account = self.get_owned_account(session, user_id=user_id, account_id=account_id)
        if req.operating_mode == OperatingMode.LIVE_TRADING:
            if req.explicit_confirmation != "CONFIRM_LIVE_TRADING":
                raise AccountValidationError(
                    "Switching to LIVE_TRADING requires explicit_confirmation='CONFIRM_LIVE_TRADING'"
                )
            cred = account.credentials
            if not (
                cred
                and cred.is_connected
                and cred.encrypted_private_key
                and cred.encrypted_api_key
                and cred.encrypted_api_secret
                and cred.encrypted_api_passphrase
            ):
                raise AccountValidationError(
                    "Cannot activate LIVE_TRADING without connected signing key and L2 API credentials"
                )
            account.live_trading_Confirmed_at = datetime.now(timezone.utc)

        old_mode = account.operating_mode
        account.operating_mode = req.operating_mode.value

        session.add(
            AccountActivityLog(
                user_id=user_id,
                trading_account_id=account.id,
                action="OPERATING_MODE_CHANGED",
                details_json={"old_mode": old_mode, "new_mode": account.operating_mode},
            )
        )
        session.flush()
        session.refresh(account)
        return _to_account_response(account)

    def set_emergency_stop(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
        req: EmergencyStopRequest,
    ) -> TradingAccountResponse:
        account = self.get_owned_account(session, user_id=user_id, account_id=account_id)
        account.emergency_stop = req.emergency_stop
        account.emergency_stop_reason = req.reason if req.emergency_stop else None
        account.trading_status = "EMERGENCY_STOPPED" if req.emergency_stop else "ACTIVE"
        if not req.emergency_stop:
            account.circuit_breaker_tripped = False
            account.consecutive_errors = 0

        session.add(
            AccountActivityLog(
                user_id=user_id,
                trading_account_id=account.id,
                action="EMERGENCY_STOP_SET" if req.emergency_stop else "EMERGENCY_STOP_CLEARED",
                details_json={"reason": req.reason},
            )
        )
        session.flush()
        session.refresh(account)
        return _to_account_response(account)

    def update_risk_limits(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
        req: RiskLimitUpdateRequest,
    ) -> RiskLimitResponse:
        account = self.get_owned_account(session, user_id=user_id, account_id=account_id)
        risk = account.risk_limits
        if risk is None:
            raise AccountValidationError("Risk limits not initialized for account")

        for field_name, val in req.model_dump(exclude_unset=True).items():
            if val is not None:
                setattr(risk, field_name, val)

        session.add(
            AccountActivityLog(
                user_id=user_id,
                trading_account_id=account.id,
                action="RISK_LIMITS_UPDATED",
                details_json=req.model_dump(exclude_unset=True, mode="json"),
            )
        )
        session.flush()
        session.refresh(risk)
        return RiskLimitResponse.model_validate(risk)

    def update_copy_strategy(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
        req: CopyStrategyUpdateRequest,
    ) -> CopyStrategyResponse:
        account = self.get_owned_account(session, user_id=user_id, account_id=account_id)
        strategy = account.copy_strategies[0] if account.copy_strategies else None
        if strategy is None:
            raise AccountValidationError("Copy strategy not initialized for account")

        updates = req.model_dump(exclude_unset=True)
        for field_name, val in updates.items():
            if val is not None or field_name == "allowed_markets":
                if field_name == "execution_mode" and hasattr(val, "value"):
                    setattr(strategy, field_name, val.value)
                else:
                    setattr(strategy, field_name, val)

        session.add(
            AccountActivityLog(
                user_id=user_id,
                trading_account_id=account.id,
                action="COPY_STRATEGY_UPDATED",
                details_json=req.model_dump(exclude_unset=True, mode="json"),
            )
        )
        session.flush()
        session.refresh(strategy)
        return CopyStrategyResponse.model_validate(strategy)

    def get_decrypted_signing_bundle(
        self,
        session: Session,
        *,
        user_id: int,
        account_id: int,
    ) -> dict[str, Optional[str]]:
        """Restricted internal method for Execution Workers ONLY. Never exposed via API."""
        account = self.get_owned_account(session, user_id=user_id, account_id=account_id)
        cred = account.credentials
        if cred is None or not cred.is_connected:
            raise AccountValidationError("Account credentials are disconnected")
        return {
            "funder_wallet_address": account.funder_wallet_address,
            "signer_wallet_address": account.signer_wallet_address,
            "signature_type": account.signature_type,
            "private_key": (
                self._vault.decrypt_secret(cred.encrypted_private_key, user_id=user_id, trading_account_id=account.id)
                if cred.encrypted_private_key
                else None
            ),
            "api_key": (
                self._vault.decrypt_secret(cred.encrypted_api_key, user_id=user_id, trading_account_id=account.id)
                if cred.encrypted_api_key
                else None
            ),
            "api_secret": (
                self._vault.decrypt_secret(cred.encrypted_api_secret, user_id=user_id, trading_account_id=account.id)
                if cred.encrypted_api_secret
                else None
            ),
            "api_passphrase": (
                self._vault.decrypt_secret(
                    cred.encrypted_api_passphrase, user_id=user_id, trading_account_id=account.id
                )
                if cred.encrypted_api_passphrase
                else None
            ),
        }
