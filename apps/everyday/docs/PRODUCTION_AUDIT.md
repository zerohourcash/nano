# Аудит готовности автономного узла

Дата последнего полного аудита: 2026-09-01. Этот документ отделяет проверенные
свойства текущего продукта от запланированных транспортов. Зелёный unit-test сам
по себе не считается доказательством сетевого требования: для P2P используются
отдельные процессы и отдельные SQLite-базы.

| Требование | Реализация | Исполняемое доказательство | Статус |
| --- | --- | --- | --- |
| Подписанная выдача по QR | canonical `everyday:item:<UUID>`, Ed25519 device-proof, Ledger V2, append-only custody commitment и восстановление holdings | `npm run test:e2e`, `npm run sync:test`, Rust QR/custody tamper tests | Проверено |
| Инвентаризация offline → owner | область `всё/склад/объект` фиксирует точный набор позиций; optional transfer-lock блокирует выдачу/возврат/передачу до завершения и синхронизируется; камера/Android bridge/фото декодируют реальный QR и принимают только GUID/номер позиции текущей сессии; создание, каждая QR-сверка и завершение — device-signed portable records | Playwright выбирает объект, включает lock, проверяет отказ возврата, декодирует PNG через `jsQR`, завершает и проверяет разблокировку/device-proof; Rust scope/lock и sync round-trip/tamper tests; `npm run smoke`, `npm run sync:test` | Проверено |
| Master-карточка ТМЦ | создание/изменение требует отдельный Ed25519-signed HTTP request; полное portable state связано с Ledger, offline-ветви сходятся детерминированно, node-signed подмена snapshot отклоняется | Rust `device_signed_item_state_rejects_a_trusted_node_rewrite`, `npm run smoke`, `npm run sync:test` | Проверено |
| Удаление без воскрешения | физические строки сохраняются; delete-wins item tombstone связан с device-signed Ledger и монотонно распространяется после offline-разрыва | `npm run smoke`, `npm run sync:test`, Rust forged-tombstone test | Проверено |
| Текстовые комментарии ТМЦ | append-only SHA-256 record связан с device-signed Ledger; offline round-trip и подмена текста доверенной нодой проверяются до импорта | Rust `portable_text_fault_and_change_branches_reject_falsification`, integrity audit | Проверено |
| Неисправности и ремонт | report/resolve — device-signed append-only branches; состояние восстанавливается из записей, concurrent offline-решения сходятся детерминированно, подмена решения отклоняется до импорта | Rust offline branch/forged-resolution test, `npm run smoke`, `npm run sync:test` | Проверено |
| Заявки на изменение | device-signed portable patch + before-state; GUID/slug/name references, concurrent accept/reject convergence, реальное применение/откат карточки, legacy adoption | Rust branch/forged-patch test, `npm run smoke`, `npm run sync:test` | Проверено |
| Прямая передача | двухфазные sender prepare / recipient accept; custody debit+credit только после подтверждения, без повторного списания склада | Rust direct-transfer/forged-recipient tests, `npm run adversarial:test` | Проверено |
| Работа без интернета | локальные Rust/SQLite/PWA, операции не требуют peer | `npm run mesh:test`, `npm run mobile:node:test` | Проверено |
| Догон после разрыва | account-chain frontier, идемпотентный store-and-forward | `npm run mesh:test`, `npm run sync:test` | Проверено |
| Конфликты/двойная выдача | обе ветви сохраняются, предмет → `needs-check`; API списка и решения ограничен активной организацией, ID чужого конфликта не обходит ACL | `npm run adversarial:test`, Rust `conflict_routes_are_scoped_to_the_callers_workspace` | Проверено |
| Подделка/replay | snapshot hash, Ed25519, nonce, portable device registry/revoke tombstone, node trust, signed monotonic sequence/scope, atomic receipt, rollback savepoint | `npm run adversarial:test`, Rust device-binding/rollback/equivocation tests, `npm run verify` | Проверено |
| Несколько организаций | scoped capability journal/CAS; versioned membership ACL и revoke tombstones | `npm run capability:test`, concurrent role/revoke в `npm run adversarial:test`, Rust merge/tamper tests | Проверено |
| Межорганизационный mesh-конверт | scoped-журналы не смешиваются; gateway identity, QR-визитка, каталог с signed rotate/revoke, inbox/outbox, opaque X25519 + HKDF-SHA256 + XChaCha20-Poly1305, Ed25519 sender proof, TTL/envelope replay/semantic transaction replay/PoW/quota, signed send/accept и обратная encrypted-квитанция с independently verified V3 device+node proof | `npm run interorg:test`: 2 процесса/БД без общего token, physical partition/restart, ciphertext inspection, delivery, accept, обратный gossip receipt, проверка V3 proof/correlation, новый envelope с тем же transaction ID и post-revoke rejection; Rust перестраивает canonical event, отвергает tamper и receipt без proof даже от trusted peer; Playwright V3 address/revoke; QR/MTU tests | Проверено для текстовых транзакций |
| Административная летопись | участники, роли, приглашения и дерево требуют device-proof и пишутся атомарно в Ledger V2; importer проверяет тип события, actor, target GUID и полноту proof | `npm run smoke`, `npm run sync:test`, Rust membership/device/wrong-target tests | Проверено |
| Жизненный цикл профиля | обновление/пароль/self-leave/delete требуют device-proof; выход блокируется при активном custody и для последнего администратора; удаление проверяет пароль, отзывает credentials и обезличивает PII, сохраняя Ledger/tombstone | Rust `self_leave_is_ledger_bound_and_custody_and_last_admin_are_guarded`, `account_deletion_verifies_password_anonymizes_and_preserves_history`, device critical-route regression | Проверено |
| Структура организации после offline-разрыва | разделы любой глубины — device-signed portable branches; parent/responsible передаются по GUID, конфликт сходится по `(depth, versionHash)`, циклы и node-signed подмена снимка запрещены | Rust `signed_organization_tree_replication_rejects_snapshot_rewrite`, API cycle test, integrity audit | Проверено |
| Подписанный акт инвентаризации | завершённая сессия выгружается как portable JSON с каноническими байтами, SHA-256, Ed25519 node-proof и ссылками на device-signed records; принимающая нода различает trusted/untrusted key, чужую организацию, missing/mismatched history; standalone CLI проверяет файл без БД/сети | Rust проверяет байты/hash/signature, подмену, неизвестный ключ, отсутствующую историю и tenant isolation; Playwright скачивает и проверяет оригинал/подделку; `npm run smoke` передаёт реальный акт отдельному CLI-процессу и проверяет exit `0/2` | Проверено |
| Справочники после offline-разрыва | склады, площадки, категории, бренды и статусы — device-signed portable branches с GUID/tombstone; карточки переносят ссылки по GUID | Rust `signed_config_survives_offline_sync_and_rejects_falsification`, integrity audit | Проверено |
| 100 узлов | разреженная топология, 100 процессов/БД, отказ и cold restart 10 процессов; журнал и CAS догоняются после восстановления | `npm run scale:test`: 100/100 за 21,3 с, leaf→root за 2,1 с, peer limit 4, RSS p95 11,9 MiB | Проверено 2026-09-01 |
| Фото и документы | двухфазный CAS ingest → маленькие signed `photo_add`/`document_add`; V3 intent связывает GUID карточки и вложения, CAS-хэш, MIME, ACL и семантические поля; chunks, resume, metadata/smart/full | Playwright реальная загрузка из браузера; `npm run sync:test` физически останавливает upstream, создаёт PDF локально, восстанавливает full-node, проверяет bytes/ACL/rollback; Rust atomic rollback и trusted-node rewrite tests | Проверено для новых вложений; старые записи явно legacy |
| Любой узел → full | смена content mode и фоновая догрузка | `npm run sync:test` | Проверено |
| Чат и wiki | V3 user intent для текста/ACL/parent/CAS attachments; chat-файлы и wiki-файлы загружаются двухфазно, старые записи explicit legacy | Playwright browser uploads, `npm run mesh:test`, `npm run sync:test` с offline chat attachment, Rust trusted-node rewrite tests | Проверено для новых сообщений и ревизий |
| Bit и бухгалтерия | кошелёк, прямой платёж и portable двухфазная продажа штучных ТМЦ и части количественной партии: seller offer → partition sync → buyer accept/reject → обратная сходимость Bit + custody | Playwright формирует предложение 4 из 6 выданных единиц и проверяет отсутствие раннего списания/custody; Rust проверяет partial holdings 6→2+4, две независимые БД, delta round-trip, балансы/ответственного, подмену цены/количества, replay, insufficient funds, stale seller и seller spoof | Проверено |
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

Последний полный локальный gate 2026-09-01: `production:audit` и
`scale:test` завершились без ошибок. Масштабный прогон поднял 100 отдельных
процессов с отдельными БД, распространил подписанные чат/выдачу и CAS-вложение,
остановил и перезапустил 10 узлов, после чего все 10 догнали журнал и файл.

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
