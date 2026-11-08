"""Out-of-band development/test email delivery sink (Fix S-03).

Ensures password-reset and email-verification tokens are NEVER returned in HTTP API responses
or written to logs, while still allowing automated tests and local development workflows to
inspect out-of-band email deliveries safely.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
import threading
from typing import Optional


@dataclass(frozen=True)
class OutboundEmailMessage:
    recipient_email: str
    subject: str
    kind: str  # "password_reset" | "email_verification"
    token: str
    sent_at: datetime


class DevMailboxSink:
    """Thread-safe in-memory outbox for out-of-band verification and password reset tokens."""

    def __init__(self) -> None:
        self._lock = threading.Lock()
        self._messages: list[OutboundEmailMessage] = []

    def send_token_email(self, *, recipient_email: str, subject: str, kind: str, token: str) -> None:
        msg = OutboundEmailMessage(
            recipient_email=recipient_email.strip().lower(),
            subject=subject,
            kind=kind,
            token=token,
            sent_at=datetime.now(timezone.utc),
        )
        with self._lock:
            if len(self._messages) >= 1000:
                self._messages.pop(0)
            self._messages.append(msg)

    def get_latest_token(self, recipient_email: str, kind: str) -> Optional[str]:
        target = recipient_email.strip().lower()
        with self._lock:
            for msg in reversed(self._messages):
                if msg.recipient_email == target and msg.kind == kind:
                    return msg.token
        return None

    def clear(self) -> None:
        with self._lock:
            self._messages.clear()


DEV_MAILBOX = DevMailboxSink()
