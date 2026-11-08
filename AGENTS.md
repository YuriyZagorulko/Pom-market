# AGENTS.md — Руководство и контекст проекта Pom-Market для AI-агентов (Cursor, Cline, Claude Code)

Данный документ содержит полную техническую документацию, архитектурный контекст, описание моделей данных, пайплайнов обработки событий и обязательные правила разработки для проекта **Pom-Market** (`/home/yuri/Desktop/my/projects/Pom-market`).

---

## 1. Project Overview (Обзор проекта)

**Pom-Market** — это self-hosted, multi-tenant платформа копитрейдинга для предсказательного рынка **Polymarket** (Polygon mainnet / CTF Exchange V2).

### Основные бизнес-функции и назначение платформы
- **Multi-tenancy (Мультитенантность)**: поддержка множества независимых зарегистрированных пользователей (`users`), где каждый пользователь изолирован на уровне приложения и на уровне СУБД (PostgreSQL `FORCE ROW LEVEL SECURITY` с не-суперпользовательской ролью `pommarket_app`).
- **Множественные торговые аккаунты**: один пользователь может подключить несколько независимых торговых аккаунтов Polymarket (`trading_accounts`) с разными типами кошельков и подписей (`EOA`, `POLY_PROXY`, `POLY_GNOSIS_SAFE`, `POLY_1271`), отдельными зашифрованными ключами (`trading_account_credentials`), отдельными балансами (`account_balances`) и индивидуальными риск-лимитами (`risk_limits`).
- **Мониторинг и копирование трейдеров-источников (`source_traders`)**: платформа рассчитана на отслеживание 2,000+ исходных кошельков Polymarket и маршрутизацию событий через `100,000+` связей «аккаунт $\leftrightarrow$ исходный трейдер» (`account_source_traders`) за время $O(1)$.
- **Три режима работы (`OperatingMode`)**:
  1. `MONITORING` (по умолчанию): сбор, нормализация и маршрутизация сделок трейдеров без выставления ордеров.
  2. `PAPER_TRADING`: симуляция исполнения (`BUY`/`SELL`) по реальному стакану (`OrderBookSnapshot`) с учётом комиссий тейкера, проскальзывания и виртуальных позиций.
  3. `LIVE_TRADING`: реальная подпись ордеров по стандарту EIP-712 (`Polymarket CTF Exchange V2`) и отправка в CLOB v2 REST API (требует явного подтверждения `CONFIRM_LIVE_TRADING`).

### Путь сигнала от сделки исходного трейдера до позиции аккаунта
1. **Сбор (`MarketDataCollector`)**: получение публичной сделки из RTDS WebSocket (`wss://ws-live-data.polymarket.com`), Data API v2 (`GET /v2/activity`) или логов смарт-контракта Polygon CTF Exchange V2 (`OrderFilled`).
2. **Нормализация и дедупликация (`EventProcessingService`)**: приведение к `NormalizedTradeEvent` и дедупликация по ключу `(tx_hash, source_wallet, token_id, side)`.
3. **Маршрутизация (`AccountRouterIndex` / `AccountRouter`)**: $O(1)$ поиск всех активных целевых аккаунтов, подписанных на `source_wallet`, с проверкой исторического порога (`source_event_ts >= mapping_created_at`) и создание задач `ExecutionJob`.
4. **Исполнение и учёт (`CopyTradingEngine` / `LiveExecutionWorkerPool`)**: проверка риск-лимитов, расчёт размера ордера, привязка к сетке центов (`$0.01`), сохранение durable-интента (`PREPARED` $\to$ `SUBMITTING`), исполнение (`FAK`), запись в `orders`, `fills`, `virtual_positions` и `account_balances`.
5. **Сверка (`ReconciliationService`)**: разрешение неопределённых (`AMBIGUOUS`, `SUBMITTING`) исходов HTTP-запросов перед любым повтором и восстановление незавершённых задач после рестарта воркера.

### Роли компонентов (Frontend, Backend, Rust Engine, Admin)
- **Backend (`backend/app/`)**: FastAPI REST & WebSocket сервер, аутентификация (Argon2id, JWT, TOTP 2FA), управление аккаунтами и крипто-хранилищем (`CredentialVault`), сервис трейдеров-источников, движок копитрейдинга, воркеры исполнения и сервис сверки.
- **Rust Engine (`rust-engine/`)**: высокопроизводительная библиотека (`rust-engine/src/lib.rs`) и HTTP-сервис (`rust-engine/src/main.rs`) на базе Tokio и `DashMap` для парсинга кадров RTDS и $O(1)$ маршрутизации. *(Примечание: в текущей реализации основная бизнес-логика и тесты выполняются через Python-сервисы в `backend/app/services/`, а `rust-engine` предоставляет библиотечные примитивы и `/health` + `/metrics` эндпоинты).*
- **Frontend (`frontend/src/`)**: веб-интерфейс на Next.js 14 + TypeScript (пользовательский дашборд, карточка торгового аккаунта, профиль исходного трейдера) и **Admin Console** (`/admin`) для пользователей с ролью `SUPER_ADMIN`.

---

## 2. Technology Stack (Фактический стек технологий)

Проверено по конфигурационным файлам репозитория (`pyproject.toml`, `Dockerfile.backend`, `frontend/package.json`, `rust-engine/Cargo.toml`, `docker-compose.yml`):

- **Backend (Python 3.12)**:
  - `FastAPI` (`0.141.1`) + `Uvicorn` (`0.52.4`) — REST API и WebSocket (`/ws/activity`).
  - `SQLAlchemy` (`2.0.52`) + `psycopg2-binary` (`2.9.12`) / `asyncpg` (`0.31.0`) — работа с БД и RLS-транзакциями.
  - `Pydantic` (`2.13.4`) + `pydantic-settings` (`2.15.0`) — валидация схем и конфигурация.
  - `PyNaCl` (`1.5.0`) — хеширование паролей `Argon2id` (`nacl.pwhash.argon2id`).
  - `cryptography` (`41.0.7`) — шифрование секретов `AES-256-GCM` и деривация ключей `HKDF-SHA256`.
  - `PyJWT` (`2.7.0`) — выпуск и проверка JWT access-токенов.
  - `ecdsa` (`0.19.2`) — криптография `secp256k1` для подписи ордеров Polymarket EIP-712 и восстановления `v in {27, 28}` (`ecrecover`).
  - `httpx` (`0.28.1`) — HTTP-клиент для взаимодействия с Polymarket API.
  - `pytest` (`9.1.1`) + `pytest-asyncio` (`1.4.0`) — автоматическое тестирование.
- **Database & Migrations**:
  - `PostgreSQL 16` (`postgres:16-alpine`) с включённым `FORCE ROW LEVEL SECURITY`.
  - `Alembic` (`1.19.1`) — миграции схемы БД (`alembic/`).
- **Rust Engine (`rust-engine/Cargo.toml`, edition 2021)**:
  - `tokio` (`1.40`), `dashmap` (`6.1`), `rust_decimal` (`1.36`), `serde` / `serde_json` (`1.0`), `chrono` (`0.4`), `reqwest` (`0.12`), `tokio-tungstenite` (`0.24`), `hmac` (`0.12`), `sha2` (`0.10`), `aes-gcm` (`0.10`), `hkdf` (`0.12`).
- **Frontend (`frontend/package.json`)**:
  - `Next.js` (`14.2.15`), `React` (`18.3.1`), `React DOM` (`18.3.1`), `TypeScript` (`5.6.2`). *(Примечание: стилизация в текущих компонентах `frontend/src/app/` выполнена через inline React styles без подключения Tailwind CSS).*
- **Infrastructure & Monitoring (`docker-compose.yml`)**:
  - `Docker Compose`, `Redis 7` (`redis:7-alpine`), `NATS 2.10 JetStream` (`nats:2.10-alpine`), `Prometheus` (`prom/prometheus:v2.54.1`), `Grafana` (`grafana/grafana:11.2.0`).

---

## 3. Architecture (Архитектура и структура директорий)

### Структура репозитория
- `backend/app/config.py` — централизованная конфигурация `Settings` из переменных окружения.
- `backend/app/db.py` — инициализация SQLAlchemy Engine, пула соединений, `SessionLocal`, хуков `after_begin` и `checkout` для PostgreSQL RLS (`pommarket_app`).
- `backend/app/models.py` — 17 ORM-моделей SQLAlchemy 2.0.
- `backend/app/schemas.py` — Pydantic v2 схемы запросов и ответов (строго без секретных полей в ответах).
- `backend/app/security.py` — Argon2id, `CredentialVault` (AES-256-GCM + HKDF + AAD), RFC-6238 TOTP, JWT и редактирование секретов (`redact_sensitive_dict`).
- `backend/app/main.py` — FastAPI приложение, зависимости авторизации (`get_current_user`, `require_super_admin`), REST роуты и WebSocket `/ws/activity`.
- `backend/app/services/` — бизнес-логика по модулям:
  - `auth_service.py` — регистрация, вход, ротация refresh-токенов, выход, сброс пароля, 2FA TOTP, блокировка пользователей.
  - `email_service.py` — out-of-band почтовый синк `DevMailboxSink` (`DEV_MAILBOX`) для безопасной доставки токенов верификации и сброса пароля вне HTTP-ответов.
  - `account_service.py` — управление торговыми аккаунтами Polymarket, проверка прав владения (`get_owned_account`), ротация/отключение ключей, смена режима (`MONITORING` / `PAPER_TRADING` / `LIVE_TRADING`), риск-лимиты и стратегии.
  - `source_trader_service.py` — привязка исходных кошельков к аккаунтам (`M:N`), индивидуальные настройки копирования, импорт/экспорт CSV.
  - `account_router.py` — потокобезопасный `AccountRouterIndex` в памяти для маршрутизации `source_wallet -> tuple[RouteTarget, ...]`.
  - `collector_service.py` — `MarketDataCollector` для парсинга кадров RTDS, ответов Data API v2 (`/v2/activity`) с пагинацией и декодирования логов Polygon `OrderFilled` V2.
  - `event_processor.py` — `EventProcessingService` для дедупликации событий и создания `ExecutionJob`.
  - `copy_engine.py` — `CopyTradingEngine`, расчёт объёма ордеров, привязка к сетке центов (`cent_grid_share_step`), симуляция исполнения по стакану, учёт `VirtualPosition` и `AccountBalance`.
  - `polymarket_signer.py` — чистая реализация `keccak256`, сборка и подпись структуры EIP-712 `Order` V2 с вычислением `recovery_v` (`27`/`28`), проверка `ecrecover` (`recover_eip712_signer_address`) и L2 HMAC-SHA256 заголовков.
  - `live_execution_worker.py` — `LiveExecutionWorkerPool`, изолированные очереди аккаунтов, `TokenBucketLimiter`, двухфазный коммит интента (`PREPARED` $\to$ `SUBMITTING`) перед отправкой в CLOB.
  - `reconciliation_service.py` — `ReconciliationService` для сверки неопределённых ордеров, восстановления после рестарта и проверки расхождений позиций.
  - `dashboard_service.py` — агрегация метрик для пользовательского и административного дашбордов.
  - `metrics_service.py` — `PlatformMetricsRegistry` (`GLOBAL_METRICS`) для сбора 7-этапной телеметрии задержек и экспорта в формате Prometheus (`/metrics`).
- `alembic/` — конфигурация миграций (`env.py`) и линейная цепочка версий (`0001_initial_multi_tenant_schema.py` $\to$ `0002_phase9_rls_and_intent.py`).
- `rust-engine/` — крейт `pommarket-engine` (`src/lib.rs`, `src/main.rs`).
- `frontend/` — клиентское приложение Next.js (`src/app/page.tsx`, `src/app/accounts/[id]/page.tsx`, `src/app/traders/[wallet]/page.tsx`, `src/app/admin/page.tsx`, `src/lib/api.ts`).
- `tests/` — полный набор автоматических тестов (`test_phase2_foundation.py` .. `test_phase8_audit.py`).

### Сервисы Docker Compose и выделенные порты разработки
В `docker-compose.yml` настроены 8 сервисов с выделенными нестандартными хост-портами (внутри контейнеров сохраняются стандартные порты):
- `web-dashboard` (`4319:3000`)
- `api-backend` (`4320:8000`)
- `rust-engine` (`4321:8080`)
- `postgres` (`5440:5432`)
- `redis` (`6390:6379`)
- `nats` (`4225:4222`, мониторинг `8225:8222`)
- `prometheus` (`9095:9090`)
- `grafana` (`3015:3000`)

---

## 4. Multi-Tenant Account, Source Trader & Position System (Устройство сущностей и связей)

В контексте Pom-Market роль «каталога и предложений» выполняет иерархия **Пользователи $\to$ Торговые аккаунты $\to$ Исходные трейдеры $\to$ Атрибутированные виртуальные позиции**:

1. **Глобальный справочник исходных трейдеров (`SourceTrader` / таблица `source_traders`)**:
   - Хранит уникальный `wallet_address` отслеживаемого публичного трейдера Polymarket, агрегированное число замеченных сделок (`total_observed_trades`), объём (`total_observed_volume_usd`) и оценку баланса (`estimated_balance_usd`).
   - Один и тот же `SourceTrader` может копироваться десятками разных пользователей и торговых аккаунтов.
2. **Торговый аккаунт пользователя (`TradingAccount` / таблица `trading_accounts`)**:
   - Принадлежит конкретному тенанту (`user_id`).
   - Различает `funder_wallet_address` (адрес прокси/Gnosis Safe, где лежат средства) и `signer_wallet_address` (EOA-ключ, подписывающий ордера), а также `signature_type` (`EOA`, `POLY_PROXY`, `POLY_GNOSIS_SAFE`, `POLY_1271`).
3. **Связь и индивидуальная конфигурация копирования (`AccountSourceTrader` / таблица `account_source_traders`)**:
   - Связывает `trading_account_id` и `source_trader_id` (`UniqueConstraint("trading_account_id", "source_trader_id")`).
   - Хранит индивидуальные настройки именно этой пары: `custom_name`, `is_enabled`, `execution_mode`, `fixed_amount_usd`, `copy_percentage`, `copy_multiplier`, `max_order_size_usd`, `min_order_size_usd`, `max_exposure_usd`, `max_slippage_pct`, `copy_buy_enabled`, `copy_sell_enabled`.
4. **Атрибутированная виртуальная позиция (`VirtualPosition` / таблица `virtual_positions`)**:
   - **Критическое правило учёта**: позиции никогда не смешиваются в один общий котёл по токену. Каждая позиция имеет уникальный ключ:
     `(trading_account_id, source_wallet, token_id, operating_mode)`.
   - Если Трейдер А и Трейдер Б оба купили один и тот же токен `YES`, а затем Трейдер А продаёт — продажа рассчитывается и списывается **только** из лота `VirtualPosition`, атрибутированного Трейдеру А, не затрагивая лот Трейдера Б.

---

## 5. Polymarket Data & Trading Integrations (Интеграции с источниками данных Polymarket)

Все интеграции с публичными и торговыми интерфейсами Polymarket сосредоточены в `backend/app/services/collector_service.py`, `backend/app/services/polymarket_signer.py` и `rust-engine/src/lib.rs`:

1. **Polymarket RTDS WebSocket (`wss://ws-live-data.polymarket.com`)**:
   - Код: `MarketDataCollector.parse_rtds_frame` (`backend/app/services/collector_service.py`) и `parse_rtds_trade_frame` (`rust-engine/src/lib.rs`).
   - Подписка формируется функцией `build_rtds_subscribe_message()` на топики `activity/trades` (сделки с атрибуцией кошелька `proxyWallet`) и `crypto_prices/update` (контрольный heartbeat-топик).
   - Формат полей: `camelCase` (`proxyWallet`, `conditionId`, `asset`, `transactionHash`, `side`, `price`, `size`, `timestamp`).
2. **Polymarket Data API v2 (`https://data-api.polymarket.com/v2/activity`)**:
   - Код: `MarketDataCollector.parse_activity_v2_response` и `MarketDataCollector.fetch_wallet_activity_v2` (`backend/app/services/collector_service.py`).
   - **Важная особенность**: устаревший v1 эндпоинт `/activity` отключён Polymarket (`HTTP 410 Gone`). Используется актуальный `/v2/activity`, возвращающий конверт `{"data": [...], "pagination": {"has_more": ..., "next_cursor": ...}}` с полями в `snake_case` (`proxy_wallet`, `transaction_hash`, `condition_id`, `token_id`, `side`, `price`, `size`, `timestamp`).
   - Поддерживается курсорная пагинация (`next_cursor`), фильтрация неторговых событий (`REDEEM`, `SPLIT`) и автоматический retry с backoff при `HTTP 429` (`Retry-After`) и `5xx`/таймаутах.
3. **Polygon CTF Exchange V2 Logs (`OrderFilled`)**:
   - Код: `MarketDataCollector.parse_polygon_v2_log` (`backend/app/services/collector_service.py`).
   - Декодирует ончейн-логи контрактов `0xE111180000d2663C0091e4f400237545B87B996B` (CTF Exchange V2) и `0xe2222d279d744050d28e00520010520000310F59` (Neg-Risk CTF Exchange V2) по `topic0 = 0xd543adfd945773f1a62f74f0ee55a5e3b9b1a28262980ba90b1a89f2ea84d8ee`, где `topics[2]` — адрес мейкера, а `data` содержит 7 слов по 32 байта (`side`, `token_id`, `maker_amount`, `taker_amount`, `fee`). Игнорирует логи с `removed: true` при реорганизации цепочки.
4. **CLOB v2 Order Book & Signing (`https://clob.polymarket.com`)**:
   - Код: `OrderBookSnapshot.from_raw` (`backend/app/services/copy_engine.py`), `build_and_sign_v2_order` и `build_l2_auth_headers` (`backend/app/services/polymarket_signer.py`).
   - **Особенность стакана CLOB**: Polymarket `/book` возвращает `asks` по убыванию цены (лучшая цена в конце массива). `OrderBookSnapshot.from_raw` принудительно сортирует `asks` по возрастанию цены, а `bids` — по убыванию.

---

## 6. Event Processing & Copy-Trading Pipeline (Жизненный цикл обработки события)

Полный конвейер от входящего события до исполнения ордера:

1. **Получение и нормализация (`MarketDataCollector`)**:
   - Извлечение и валидация `source_wallet`, `token_id`, `side` (`BUY`/`SELL`), `price`, `size`, `notional_usd = price * size`, `tx_hash` и временных меток `source_event_ts`, `event_received_ts`, `event_normalized_ts`.
2. **Дедупликация (`EventProcessingService.process_event`)**:
   - Проверка по in-memory множеству `_seen_keys` и уникальному индексу БД `uq_source_trade_events_dedup` на кортеж `(tx_hash, source_wallet, token_id, side)`. Одно и то же событие, пришедшее из RTDS, `/v2/activity` и Polygon logs, обрабатывается ровно один раз.
3. **Защита от ретроактивного копирования истории**:
   - При создании `ExecutionJob` проверяется `int(source_event_ts.timestamp()) >= int(mapping_created_at.timestamp())`. Сделки, совершённые трейдером до момента добавления его кошелька в аккаунт, никогда не копируются.
4. **Маршрутизация (`AccountRouterIndex.match_targets`)**:
   - Для каждого подписанного целевого аккаунта создаётся запись `ExecutionJob`:
     - если аккаунт в режиме `MONITORING`, задача сразу получает статус `COMPLETED` (`reason="monitoring_mode_no_order"`) без создания ордера;
     - если аккаунт в `PAPER_TRADING` или `LIVE_TRADING`, задача ставится в статус `QUEUED`.
5. **Проверки безопасности и риск-лимитов (`CopyTradingEngine.execute_job`)**:
   - Проверка глобального (`global_emergency_stop`) и аккаунтного (`account.emergency_stop`) стоп-крана, а также `circuit_breaker_tripped`.
   - Защита от устаревших данных (`age_secs > risk.max_stale_data_seconds`).
   - Проверка белого списка рынков (`strategy.allowed_markets`).
   - Проверка дневного лимита убытков (`daily_loss_stop_latched` / `max_daily_loss_usd`).
   - Проверка частоты сделок (`max_executions_per_interval` за `execution_interval_seconds`).

---

## 7. Pricing, Sizing Modes, Fees & Precision Rules (Ценообразование, комиссии и точность)

1. **Режимы расчёта объёма (`ExecutionMode` в `CopyTradingEngine.compute_buy_target_notional`)**:
   - `FIXED_AMOUNT`: `fixed_amount_usd * copy_multiplier`.
   - `FIXED_PERCENTAGE`: `source_notional_usd * copy_percentage * copy_multiplier`.
   - `PROPORTIONAL_BALANCE`: `source_notional_usd * (destination_balance_usd / source_estimated_balance_usd) * copy_multiplier`.
   - `MAX_ORDER_SIZE`: `max_order_size_usd`.
   - Итоговый объём ограничивается сверху `min(mapping.max_order_size_usd, risk.max_order_size_usd, strategy.max_order_size_usd)` и проверяется снизу на минимум биржи `$1.00` (`EXCHANGE_MIN_NOTIONAL_USD`).
2. **Привязка к тикам и сетке центов (`snap_price_to_tick` и `snap_shares_to_cent_grid`)**:
   - Лимитная цена покупки: `floor(min(source_price * (1 + max_slippage), 0.999) / tick_size) * tick_size`.
   - Лимитная цена продажи: `ceil(max(source_price * (1 - max_slippage), 0.001) / tick_size) * tick_size`.
   - **Критическое правило точности Polymarket (`Fix P-03`)**: нотионал ордера (`shares * limit_price`) обязан иметь не более 2 знаков после запятой (кратность `$0.01`, т.е. `10,000` базовых единиц), а количество акций `shares` — не более 5 знаков после запятой. Функция `cent_grid_share_step(limit_price)` вычисляет точный шаг акций через `gcd(mantissa, 10**(scale + 3))`, а `validate_polymarket_order_amounts()` блокирует подпись любого ордера, нарушающего это правило.
3. **Комиссия тейкера (`compute_taker_fee_usd`)**:
   - Рассчитывается одинаково в `PAPER_TRADING` и `LIVE_TRADING` по нелинейной формуле Polymarket:
     $$\text{fee\_usd} = \text{rate} \times (p \times (1 - p))^{\text{exponent}} \times \text{shares}$$
   - В `AccountBalance` при покупке списывается `notional_usd + fee_usd`, при продаже зачисляется `notional_usd - fee_usd`, а реализованный PnL равен `notional_usd - released_cost_basis - fee_usd`.

---

## 8. Virtual Position & Inventory Management (Управление позициями и выходами)

1. **Учёт `source_observed_shares` (`Fix E-01`)**:
   - Поле `VirtualPosition.source_observed_shares` увеличивается **только** в `apply_confirmed_fill()`, когда ордер на покупку (`BUY`) реально исполнился (при частичном исполнении — пропорционально `filled_shares / requested_size_shares`).
   - Пропущенные или отклонённые по риск-лимитам/проскальзыванию/балансу покупки (`SKIPPED`/`REJECTED`) **не изменяют** `source_observed_shares`.
2. **Пропорциональный выход (`SELL`) и накопление мелких долей (`pending_exit_shares`)**:
   - Когда исходный трейдер продаёт `source_sold` акций, доля выхода вычисляется как `exit_fraction = min(1.0, source_sold / source_observed_shares)`.
   - Желаемое число акций к продаже: `desired_shares = min(open_shares, open_shares * exit_fraction + pending_exit_shares)`.
   - Если `requested_shares * floor_price < $1.00` (меньше минимального ордера биржи) или в стакане нет ликвидности выше `floor_price`, продажа не завышается искусственно, а неисполненный объём сохраняется в `vpos.pending_exit_shares` до следующей продажи.
   - Система гарантирует (`ck_virtual_positions_non_negative_shares`), что никогда не будет продано больше, чем `vpos.open_shares` данного исходного трейдера.

---

## 9. Live Execution, Crash Recovery & Reconciliation (Исполнение и отказоустойчивость)

1. **Двухфазная фиксация интента перед отправкой ордера (`Fix E-03` в `LiveExecutionWorkerPool.execute_live_order`)**:
   - Перед вызовом внешнего HTTP `POST /order` воркер строит детерминированный EIP-712 ордер (`client_order_id`, `eip712_order_hash`, `signed_payload_json`), сохраняет его в статус `PREPARED` (`session.commit()`), а затем переводит в статус `SUBMITTING` и снова выполняет `session.commit()`.
   - Только после фиксации `SUBMITTING` в PostgreSQL отправляется сетевой запрос в Polymarket.
2. **Защита от двойного исполнения при таймаутах (`AMBIGUOUS`)**:
   - Если HTTP-запрос завершился таймаутом (`504` / обрыв сети) или воркер упал сразу после отправки `POST /order`, ордер **никогда** не отправляется повторно вслепую.
   - При перезапуске `ReconciliationService.recover_unfinished_jobs_after_restart()` сначала запрашивает фактический статус всех ордеров в состояниях `SUBMITTING`, `ACKNOWLEDGED`, `AMBIGUOUS`, `SUBMITTED` у Polymarket (`reconcile_ambiguous_orders()`) и идемпотентно проводит `Fill` (если ордер исполнился) либо помечает `REJECTED`/`CANCELLED` (если биржа его не исполнила).

---

## 10. Dashboard & Administrative Operations (Дашборд и администрирование)

1. **Реализованные API-эндпоинты (`backend/app/main.py`)**:
   - Аутентификация и профиль: `/api/v1/auth/register`, `/api/v1/auth/login`, `/api/v1/auth/refresh`, `/api/v1/auth/logout`, `/api/v1/auth/verify-email`, `/api/v1/auth/password-reset/initiate`, `/api/v1/auth/password-reset/confirm`, `/api/v1/auth/totp/setup`, `/api/v1/auth/totp/enable`, `/api/v1/auth/me`.
   - Торговые аккаунты: `GET/POST /api/v1/accounts`, `GET /api/v1/accounts/{id}`, `GET /api/v1/accounts/{id}/detail`, `POST /api/v1/accounts/{id}/credentials/rotate`, `POST /api/v1/accounts/{id}/disconnect`, `POST /api/v1/accounts/{id}/mode`, `POST /api/v1/accounts/{id}/emergency-stop`, `PATCH /api/v1/accounts/{id}/risk-limits`, `PATCH /api/v1/accounts/{id}/copy-strategy`.
   - Исходные трейдеры: `GET/POST /api/v1/accounts/{id}/source-traders`, `PATCH/DELETE /api/v1/accounts/{id}/source-traders/{source_trader_id}`, `GET /api/v1/accounts/{id}/source-traders/export`, `GET /api/v1/source-traders/{wallet_address}`.
   - Дашборд и мониторинг: `GET /api/v1/dashboard/summary`, `GET /health`, `GET /metrics`, `WS /ws/activity`.
   - Администрирование (`SUPER_ADMIN`): `GET /api/v1/admin/overview`, `POST /api/v1/admin/users/{id}/suspend`, `POST /api/v1/admin/emergency-stop`.
2. **Известные ограничения текущей реализации**:
   - Протокол `PolymarketExchangeGateway` (`backend/app/services/live_execution_worker.py`) спроектирован для внедрения HTTP-клиента или тестового стаба; фоновый демон непрерывного чтения RTDS WebSocket в едином цикле с NATS JetStream ещё не вынесен в отдельный воркер-процесс.
   - Внутренние страницы Next.js (`frontend/src/app/accounts/[id]/page.tsx`, `frontend/src/app/traders/[wallet]/page.tsx`, `frontend/src/app/admin/page.tsx`) содержат UI-разметку с демонстрационным состоянием, тогда как главная страница (`frontend/src/app/page.tsx`) и клиент `PlatformApiClient` (`frontend/src/lib/api.ts`) интегрированы с REST/WS API.

---

## 11. Database & Migrations (База данных и миграции)

- **Расположение моделей**: `backend/app/models.py` (17 таблиц: `users`, `user_roles`, `user_sessions`, `trading_accounts`, `trading_account_credentials`, `source_traders`, `account_source_traders`, `copy_strategies`, `risk_limits`, `source_trade_events`, `execution_jobs`, `orders`, `fills`, `virtual_positions`, `account_balances`, `account_activity_logs`, `system_settings`).
- **Alembic миграции**: `alembic/versions/` (`0001_initial_multi_tenant_schema.py` $\to$ `0002_phase9_rls_and_intent.py`).
- **Обязательные правила БД**:
  - Поддерживать строго **одну линейную историю миграций и один head** (`alembic heads` должен возвращать ровно одну ревизию).
  - Приложение всегда подключается под ролью `pommarket_app` (`NOSUPERUSER NOBYPASSRLS`), а миграции запускаются через `MIGRATION_DATABASE_URL` под владельцем схемы `pommarket`.
  - Запрещено вручную менять постоянную схему или обходить миграции прямыми SQL-скриптами.

---

## 12. Development & Deployment (Разработка, тестирование и запуск)

### Выделенные порты локальной разработки
- Frontend (Next.js): `http://localhost:4319` (`FRONTEND_PORT=4319`)
- Backend (FastAPI): `http://localhost:4320` (`BACKEND_PORT=4320`, Swagger: `http://localhost:4320/docs`, WebSocket: `ws://localhost:4320/ws/activity`)
- Rust Engine: `http://localhost:4321` (`RUST_ENGINE_PORT=4321`)
- PostgreSQL 16: `localhost:5440` (`POSTGRES_PORT=5440`)
- Redis 7: `localhost:6390` (`REDIS_PORT=6390`)
- NATS JetStream: `localhost:4225` / мониторинг `http://localhost:8225`
- Prometheus: `http://localhost:9095`
- Grafana: `http://localhost:3015`

### Команды для работы
```bash
# Проверка единственного head и применение миграций
alembic heads
alembic upgrade head

# Запуск полного набора тестов (22 теста против PostgreSQL 16 на порту 5440)
pytest -v

# Запуск инфраструктуры и сервисов через Docker Compose
docker compose up -d
```

---

## 13. Important Development Rules (Обязательные правила для AI-агентов и разработчиков)

1. **Минимальные и точечные изменения**: не выполнять ненужный рефакторинг и не переписывать работающие модули без явного запроса.
2. **Предварительное исследование**: перед любым изменением изучить существующую реализацию в `backend/app/`, `alembic/` и `tests/`.
3. **Запрет дублирования бизнес-логики**: не создавать параллельные механизмы маршрутизации, расчёта комиссий, шифрования секретов или проверки тенанта там, где уже действуют `AccountRouterIndex`, `CopyTradingEngine`, `CredentialVault` и `set_tenant_context`.
4. **Строгая мультитенантная изоляция**: каждый пользовательский ресурс обязан быть привязан к `user_id` и защищён как проверкой на уровне сервиса (`get_owned_account`), так и политикой PostgreSQL RLS (`pommarket_app`).
5. **Безопасность секретов и ордеров**:
   - Никогда не возвращать приватные ключи, API-секреты, пароли, токены сброса пароля или верификации в HTTP-ответах или логах.
   - Никогда не отправлять повторный live-ордер при HTTP-таймауте (`AMBIGUOUS` / `SUBMITTING`) без предварительной сверки через `ReconciliationService`.
   - Никогда не включать реальную торговлю (`LIVE_TRADING`) и не отправлять реальные транзакции на Polymarket во время разработки или тестирования.
6. **Миграции и тесты**:
   - Все изменения схемы БД оформлять исключительно через линейные миграции Alembic.
   - Каждое изменение бизнес-логики или исправление бага сопровождать регрессионными тестами в `tests/` и прогоном `pytest -v`.
7. **Операционные запреты**:
   - Не выполнять автоматический `git commit` или `git push` без прямого разрешения пользователя.
   - Не изменять production-окружение и не запускать реальные боевые импорты/торговлю без разрешения.
