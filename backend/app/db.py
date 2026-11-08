"""Database engine, session management, and PostgreSQL Row-Level Security (RLS) context.

Fix S-01:
- Runtime application connections authenticate as the dedicated non-superuser role `pommarket_app`
  (`NOSUPERUSER`, `NOBYPASSRLS`) with `FORCE ROW LEVEL SECURITY` enabled on all tenant tables.
- Every SQLAlchemy `Session` stores its bound tenant context in `session.info` and automatically
  applies `app.current_user_id` and `app.is_super_admin` at the start of every transaction (`after_begin`),
  including transactions started after `session.commit()` or `session.rollback()`.
- Connection pool `reset` / `checkout` events clear any previous session variables so pooled connections
  can never leak tenant context across requests or workers.
"""

from __future__ import annotations

from collections.abc import Generator
from contextlib import contextmanager
from typing import Optional

from sqlalchemy import create_engine, event, text
from sqlalchemy.orm import DeclarativeBase, Session, sessionmaker

from backend.app.config import get_settings


class Base(DeclarativeBase):
    pass


_engine = None
_SessionLocal = None


def _apply_session_rls_vars(session: Session, connection) -> None:
    """Apply `app.current_user_id` and `app.is_super_admin` to the active PostgreSQL transaction."""
    if connection.dialect.name != "postgresql":
        return
    user_id = session.info.get("tenant_user_id")
    is_super_admin = bool(session.info.get("tenant_is_super_admin", False))
    uid_val = str(int(user_id)) if user_id is not None else ""
    admin_val = "true" if is_super_admin else "false"
    # If the connection happens to be the migration superuser (e.g. during schema setup), drop to pommarket_app
    connection.execute(text("SET LOCAL ROLE pommarket_app"))
    connection.execute(
        text(
            "SELECT set_config('app.current_user_id', :uid, true), "
            "set_config('app.is_super_admin', :adm, true)"
        ),
        {"uid": uid_val, "adm": admin_val},
    )


@event.listens_for(Session, "after_begin")
def _on_session_after_begin(session: Session, transaction, connection) -> None:
    """Automatically establish the session's tenant context on every new transaction (survives commit/rollback)."""
    _apply_session_rls_vars(session, connection)


def get_engine(database_url: Optional[str] = None):
    global _engine
    url = database_url or get_settings().database_url
    if _engine is None or str(_engine.url) != url:
        eng = create_engine(
            url,
            pool_pre_ping=True,
            pool_size=20,
            max_overflow=30,
            future=True,
        )

        @event.listens_for(eng, "checkout")
        def _on_pool_checkout(dbapi_connection, connection_record, connection_proxy) -> None:
            cursor = dbapi_connection.cursor()
            try:
                cursor.execute(
                    "SELECT set_config('app.current_user_id', '', false), "
                    "set_config('app.is_super_admin', 'false', false)"
                )
            except Exception:
                pass
            finally:
                cursor.close()

        _engine = eng
    return _engine


def get_session_factory(database_url: Optional[str] = None) -> sessionmaker[Session]:
    global _SessionLocal
    engine = get_engine(database_url)
    if _SessionLocal is None or _SessionLocal.kw.get("bind") is not engine:
        _SessionLocal = sessionmaker(bind=engine, autoflush=False, autocommit=False, expire_on_commit=False)
    return _SessionLocal


def set_tenant_context(session: Session, user_id: Optional[int], is_super_admin: bool = False) -> None:
    """Bind tenant context to `session.info` and immediately apply it to the current PostgreSQL transaction."""
    session.info["tenant_user_id"] = int(user_id) if user_id is not None else None
    session.info["tenant_is_super_admin"] = bool(is_super_admin)

    dialect = session.bind.dialect.name if session.bind is not None else ""
    if dialect == "postgresql":
        uid_val = str(int(user_id)) if user_id is not None else ""
        admin_val = "true" if is_super_admin else "false"
        session.execute(text("SET LOCAL ROLE pommarket_app"))
        session.execute(
            text(
                "SELECT set_config('app.current_user_id', :uid, true), "
                "set_config('app.is_super_admin', :adm, true)"
            ),
            {"uid": uid_val, "adm": admin_val},
        )


@contextmanager
def tenant_session(
    user_id: Optional[int] = None,
    is_super_admin: bool = False,
    database_url: Optional[str] = None,
) -> Generator[Session, None, None]:
    """Transactional database session scoped to a specific tenant (or super_admin)."""
    factory = get_session_factory(database_url)
    session = factory()
    try:
        set_tenant_context(session, user_id=user_id, is_super_admin=is_super_admin)
        yield session
        session.commit()
    except Exception:
        session.rollback()
        raise
    finally:
        session.info.clear()
        session.close()


def get_db() -> Generator[Session, None, None]:
    factory = get_session_factory()
    session = factory()
    try:
        set_tenant_context(session, user_id=None, is_super_admin=False)
        yield session
    finally:
        session.info.clear()
        session.close()
