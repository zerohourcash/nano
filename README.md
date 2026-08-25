# Bit Community

Offline-first платформа для автономной организации: локальная валюта Bit, продажи, двойная бухгалтерия, инвентаризация и материальная ответственность, управление членством и зашифрованный mesh-чат. Система проектируется без обязательного центрального сервера; интернет ускоряет синхронизацию, но не является источником истины.

Статус: активная разработка backend-протокола. Это ещё не прошедший внешний аудит production-релиз.

## Архитектура

- `bit-core`: детерминированный block-lattice, Ed25519, роли, governance, Bit, имущество, продажи и бухгалтерские проекции.
- `bit-chat`: подписанные каналы, XChaCha20-Poly1305, антиспам, файлы и сигнализация будущих WebRTC-звонков.
- `bit-knowledge`: подписанная локальная база знаний с revision DAG, offline-конфликтами, merge и ссылками на изображения/blobs.
- `bit-agent`: безопасная граница локальной нейросети: структурированные предложения, capability-политики, preview и запрет автономной подписи критических действий.
- `bit-node`: HTTP/LAN-нода и SQLite WAL с полной перепроверкой ledger при старте.
- `src`, `public`, `test`: ранний JavaScript/PWA-прототип, сохранённый для регрессии UX и сетевых сценариев.

Один телефон может состоять в нескольких организациях. Данные, роли, балансы и ключи-псевдонимы разделяются по `community_id`, чтобы профили нельзя было связать между организациями.

Подробнее: [product scope](docs/PRODUCT-SCOPE.md), [Rust architecture](docs/ADR-001-rust-architecture.md), [mesh chat](docs/ADR-002-chat.md), [resilience references](docs/ADR-003-resilience-and-references.md), [local AI](docs/ADR-004-local-ai.md), [transport contract](docs/TRANSPORT.md).

## Требования

- Rust stable 1.85+;
- Node.js 20+ — только для тестов reference-прототипа;
- `curl` — для smoke-теста HTTP-ноды.

## Полная проверка

```bash
bash scripts/quality.sh
```

Quality gate выполняет:

- `rustfmt --check`;
- `clippy -D warnings` для всех targets/features;
- Rust unit, regression, cryptographic, governance, persistence и corruption tests;
- JavaScript unit/regression tests;
- аудит npm-зависимостей;
- deployment smoke-test, создание актива и восстановление после перезапуска.

## Локальный запуск

Сгенерируйте случайный токен длиной не менее 24 символов и не сохраняйте его в Git:

```bash
export BIT_ADMIN_TOKEN="$(openssl rand -hex 32)"
export BIT_DATA_DIR="$PWD/bit-data"
export BIT_LISTEN="0.0.0.0:8787"
cargo run -p bit-node
```

Проверка:

```bash
curl http://127.0.0.1:8787/health
curl -X POST http://127.0.0.1:8787/v1/admin/assets \
  -H "Authorization: Bearer $BIT_ADMIN_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"name":"Дрель","serial":"DR-001","location":"Склад А","value_minor":159900}'
curl http://127.0.0.1:8787/v1/state
```

Телефон в той же Wi-Fi/LAN-сети обращается к `http://IP-КОМПЬЮТЕРА:8787`. Для внешней сети перед production-развёртыванием обязателен TLS reverse proxy; административный HTTP API нельзя публиковать напрямую.

## Реализованные инварианты

- Один владелец ключа продолжает свою account-chain.
- Неизвестная версия протокола отклоняется.
- Bit использует целые minor units; переплата и отрицательные суммы запрещены.
- Перевод Bit завершается парой `send/receive`.
- Передача инструмента завершается парой `custody offer/accept`.
- Бухгалтерская запись принимается только при равенстве дебета и кредита.
- После появления нескольких управляющих членство требует кворума не менее ⅔ Admin/Auditor.
- Чат не блокирует финансовый ledger и имеет собственные frontiers/retention.
- Файлы шифруются, делятся на проверяемые чанки по 64 КиБ и могут докачиваться с разных узлов.
- Антиспам ограничивает rate, TTL, hops, размеры, дубликаты и неавторизованный трафик.
- База знаний сохраняет обе параллельные offline-редакции и требует явной подписанной merge-ревизии для публикации единой версии.
- Локальная модель не получает ключ подписи: платежи, передача имущества, членство, роли и удаление аудита всегда требуют точного preview и подтверждения человеком.

## Git и релизы

Важные работающие изменения фиксируются отдельными локальными коммитами после успешного `scripts/quality.sh`. `target`, базы, ключи, токены, скачанные reference-репозитории и `node_modules` не входят в историю.

## Ограничения текущего этапа

Ещё предстоят libp2p/frontier-репликация, хранение конфликтующих fork-ветвей, M-of-N финализация финансовых блоков, полноценное device enrollment, HTTP API всех операций, UniFFI, Kotlin/Swift оболочки, Nearby/BLE/Meshtastic-адаптеры, Signal-compatible direct messages, WebRTC и внешний аудит безопасности.

Официальные исходники Nano V28.2 и документация скачаны локально в игнорируемый каталог `reference/` только для исследования. Проект не совместим с XNO и не копирует AGPL-код Erachain.
