# Аудит готовности автономного узла

Дата последнего полного аудита: 2026-09-01. Этот документ отделяет проверенные
свойства текущего продукта от запланированных транспортов. Зелёный unit-test сам
по себе не считается доказательством сетевого требования: для P2P используются
отдельные процессы и отдельные SQLite-базы.

| Требование | Реализация | Исполняемое доказательство | Статус |
| --- | --- | --- | --- |
| Подписанная выдача по QR | canonical `everyday:item:<UUID>`, Ed25519 device-proof, Ledger V2 | `npm run test:e2e`, Rust `qr_lookup…`, `device::tests` | Проверено |
| Работа без интернета | локальные Rust/SQLite/PWA, операции не требуют peer | `npm run mesh:test`, `npm run mobile:node:test` | Проверено |
| Догон после разрыва | account-chain frontier, идемпотентный store-and-forward | `npm run mesh:test`, `npm run sync:test` | Проверено |
| Конфликты/двойная выдача | обе ветви сохраняются, предмет → `needs-check` | `npm run adversarial:test` | Проверено |
| Подделка/replay | snapshot hash, Ed25519, nonce, trust registry, signed monotonic sequence/scope, atomic receipt, rollback savepoint | `npm run adversarial:test`, Rust rollback/equivocation test, `npm run verify` | Проверено |
| Несколько организаций | scoped capability journal/CAS; versioned membership ACL и revoke tombstones | `npm run capability:test`, concurrent role/revoke в `npm run adversarial:test`, Rust merge/tamper tests | Проверено |
| Административная летопись | участники, роли, приглашения и дерево требуют device-proof и пишутся атомарно в Ledger V2 | `npm run smoke`, `npm run sync:test`, Rust membership/device tests | Проверено |
| 100 узлов | разреженная топология, 100 процессов/БД, отказ 10 процессов | `npm run scale:test` | Проверено 2026-09-01 |
| Фото и документы | SHA-256 CAS, chunks, resume, metadata/smart/full | `npm run sync:test`, `npm run scale:test` | Проверено |
| Любой узел → full | смена content mode и фоновая догрузка | `npm run sync:test` | Проверено |
| Чат и wiki | ledger-bound chat; revision DAG и CAS ACL | `npm run mesh:test`, `npm run adversarial:test`, Rust tests | Проверено |
| Bit и бухгалтерия | двойная запись, deterministic conflict reconciliation | `npm run adversarial:test`, Rust accounting tests | Проверено |
| Android | тот же Rust backend, Keystore, private UI/sync-only LAN | APK verifier, `npm run mobile:node:test` | Проверено ARM64/x86_64 |
| Смена Wi‑Fi/hotspot | NetworkCallback → JNI, динамический HMAC-анонс | Android contract, `npm run discovery:test` | Проверено на host/JNI build |
| SQLCipher | отдельный feature, обязательный ключ, wrong-key rejection | `npm run encrypted-db:test` | Проверено Linux |
| Backup/restore | online `.backup`, шифрование, integrity-check, новый target | `npm run backup:restore:test` | Проверено Linux |
| Потоковый transport core | MTU frames, out-of-order, duplicate, missing ranges, SHA-256, Android JNI | Rust `stream_transport::tests`, encrypted bundle round-trip, APK symbols | Проверено |
| Android BLE GATT | foreground advertiser/server + scanner/client, MTU/retry, bounded crash-safe ciphertext spool, authorized import handoff, persistent diagnostics | Rust API regression, Android compile/lint, API 34 emulator instrumentation gate, APK | Реализовано; KVM CI gate настроен, но ещё не зафиксирован внешний успешный run; RF-тест на двух телефонах не выполнен |

## Release gates

Быстрый обязательный gate:

```bash
npm run production:audit
```

Перед крупным релизом дополнительно запускается тяжёлый `npm run scale:test` и
собирается подписанный Android release APK. Debug APK доказывает сборку и
содержимое, но не заменяет секретный keystore владельца приложения.

## Не закрыто и поэтому не заявляется production-ready

- Android BLE GATT adapter и foreground lifecycle реализованы, но ещё не
  проверены между двумя физическими телефонами с убийством Activity/процесса.
  Bluetooth Mesh managed flooding и LoRa radio adapter
  пока не реализованы. Общий bounded MTU framing, out-of-order сборка и resume
  готовы и тестируются; без IP также работает
  зашифрованный store-and-forward bundle через системный Bluetooth/Wi‑Fi
  Direct/USB Share, но это ручной перенос, не фоновый BLE gossip.
- iOS native shell и его Keychain/lifecycle тест отсутствуют; на iOS доступна
  PWA, но она не является полной фоновой нодой.
- Голосовые звонки и локальная LLM относятся к последующим модулям и не входят
  в проверенный backend-релиз.
- Независимый внешний криптографический аудит и platform penetration test ещё
  не проводились. Встроенные adversarial-тесты не заменяют такой аудит.

Пока хотя бы один обязательный для конкретного внедрения пункт из этого раздела
нужен заказчику, релиз следует называть тестируемым production candidate, а не
окончательно сертифицированным продуктом.
