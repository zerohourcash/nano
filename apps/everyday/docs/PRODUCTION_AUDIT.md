# Аудит готовности автономного узла

Дата последнего полного аудита: 2026-09-01. Этот документ отделяет проверенные
свойства текущего продукта от запланированных транспортов. Зелёный unit-test сам
по себе не считается доказательством сетевого требования: для P2P используются
отдельные процессы и отдельные SQLite-базы.

| Требование | Реализация | Исполняемое доказательство | Статус |
| --- | --- | --- | --- |
| Обязательная подписанная выдача по QR | tenant-bound QR V2 обязателен в single/bulk API; сервер проверяет Ed25519 node-proof, доверие, организацию и item binding, legacy запрещён, каждая штучная единица сканируется отдельно. Точное тело коммитится device-proof в Ledger V3, публичная история раскрывает только SHA-256 QR proof; выпуск бирки требует `editItems`. Append-only custody восстанавливает holdings. Физический клон подлинной бирки криптографически не обнаружим | `npm run smoke` проверяет no-QR/wrong/tamper/ACL/digest; Playwright декодирует реальный PNG и выдаёт через UI; `npm run sync:test`, `npm run mesh:test`, `npm run adversarial:test`, Rust QR foreign-tenant/rebind/tamper/custody tests | Проверено |
| Инвентаризация offline → owner | область `всё/склад/объект` фиксирует точный набор позиций; optional transfer-lock блокирует выдачу/возврат/передачу до завершения и синхронизируется; камера/Android bridge/фото декодируют реальный QR и принимают только GUID/номер позиции текущей сессии; создание, каждая QR-сверка и завершение — device-signed portable records | Playwright выбирает объект, включает lock, проверяет отказ возврата, декодирует PNG через `jsQR`, завершает и проверяет разблокировку/device-proof; Rust scope/lock и sync round-trip/tamper tests; `npm run smoke`, `npm run sync:test` | Проверено |
| Master-карточка ТМЦ | создание/изменение требует отдельный Ed25519-signed HTTP request; полное portable state связано с Ledger, offline-ветви сходятся детерминированно, node-signed подмена snapshot отклоняется | Rust `device_signed_item_state_rejects_a_trusted_node_rewrite`, `npm run smoke`, `npm run sync:test` | Проверено |
| Удаление без воскрешения | физические строки сохраняются; delete-wins item tombstone связан с device-signed Ledger и монотонно распространяется после offline-разрыва | `npm run smoke`, `npm run sync:test`, Rust forged-tombstone test | Проверено |
| Текстовые комментарии ТМЦ | append-only SHA-256 record связан с device-signed Ledger; offline round-trip и подмена текста доверенной нодой проверяются до импорта | Rust `portable_text_fault_and_change_branches_reject_falsification`, integrity audit | Проверено |
| Неисправности и ремонт | report/resolve — device-signed append-only branches; состояние восстанавливается из записей, concurrent offline-решения сходятся детерминированно, подмена решения отклоняется до импорта | Rust offline branch/forged-resolution test, `npm run smoke`, `npm run sync:test` | Проверено |
| Заявки на изменение | device-signed portable patch + before-state; GUID/slug/name references, concurrent accept/reject convergence, реальное применение/откат карточки, legacy adoption | Rust branch/forged-patch test, `npm run smoke`, `npm run sync:test` | Проверено |
| Прямая передача | двухфазные sender prepare / recipient accept; custody debit+credit только после подтверждения, без повторного списания склада | Rust direct-transfer/forged-recipient tests, `npm run adversarial:test` | Проверено |
| Работа без интернета | локальные Rust/SQLite/PWA, операции не требуют peer | `npm run mesh:test`, `npm run mobile:node:test` | Проверено |
| Догон после разрыва | account-chain frontier, идемпотентный store-and-forward | `npm run mesh:test`, `npm run sync:test` | Проверено |
| Антиспам внутреннего чата | device-proof, 4000 символов, 20 сообщений/мин; CAS-файл требует одноразовый hourly grant на workspace/user/message/hash, точный MIME и имеет persisted квоты 10 файлов/64 МБ в минуту, 1000/512 МБ в сутки на участника, 2 ГБ/сутки на организацию. Grant погашается атомарно с Ledger V3; чужой UUID/replay/duplicate отвергаются | `npm run spam:test` через реальный HTTP проверяет 11-й blob до CAS, SQLite counts и сохранение лимитов после restart; Playwright отправляет настоящий файл; `npm run sync:test` переносит offline CAS intent; Rust проверяет member-without-create, чужой/повторный grant, MIME и квоту | Проверено |
| Конфликты/двойная выдача | обе ветви сохраняются, предмет → `needs-check`; API списка и решения ограничен активной организацией, ID чужого конфликта не обходит ACL | `npm run adversarial:test`, Rust `conflict_routes_are_scoped_to_the_callers_workspace` | Проверено |
| Подделка/replay | snapshot hash, Ed25519, nonce, portable device registry/revoke tombstone, node trust, signed monotonic sequence/scope, atomic receipt, rollback savepoint | `npm run adversarial:test`, Rust device-binding/rollback/equivocation tests, `npm run verify` | Проверено |
| Несколько организаций | scoped capability journal/CAS; chat/wiki/item/custody/fault/writeoff manifests и blob ACL вычисляются независимо, знание чужого SHA-256 не даёт файл; versioned membership ACL и revoke tombstones | `npm run capability:test`: два bearer и два full-peer получают только свои workspace, chat CAS и фото списания, чужой blob возвращает 403; concurrent role/revoke в `npm run adversarial:test`; Rust scope/import/CAS tests | Проверено |
| Межорганизационный mesh-конверт | scoped-журналы не смешиваются; gateway identity, QR-визитка, каталог с signed rotate/revoke, inbox/outbox, opaque X25519 + HKDF-SHA256 + XChaCha20-Poly1305, Ed25519 sender proof, TTL/envelope replay/semantic transaction replay/PoW/quota, signed send/accept и обратная encrypted-квитанция с independently verified V3 device+node proof. Новый inbox хранит source envelope; полный аудит повторно открывает AEAD и сверяет точный payload и V3 accept | `npm run interorg:test`: 2 процесса/БД без общего token, physical partition/restart, ciphertext inspection, локальная SQL-подмена расшифрованного body, восстановление, delivery/accept, обратный gossip receipt, V3 proof/correlation, replay и post-revoke rejection; Rust canonical/tamper tests; Playwright V3 address/revoke; QR/MTU tests | Проверено для новых текстовых транзакций; старый inbox explicit legacy |
| Административная летопись | участники, роли, приглашения и дерево требуют device-proof и пишутся атомарно в Ledger V2; importer проверяет тип события, actor, target GUID и полноту proof | `npm run smoke`, `npm run sync:test`, Rust membership/device/wrong-target tests | Проверено |
| Жизненный цикл профиля | обновление/пароль/self-leave/delete требуют device-proof; выход блокируется при активном custody и для последнего администратора; удаление проверяет пароль, отзывает credentials и обезличивает PII, сохраняя Ledger/tombstone | Rust `self_leave_is_ledger_bound_and_custody_and_last_admin_are_guarded`, `account_deletion_verifies_password_anonymizes_and_preserves_history`, device critical-route regression | Проверено |
| Структура организации после offline-разрыва | разделы любой глубины — device-signed portable branches; parent/responsible передаются по GUID, конфликт сходится по `(depth, versionHash)`, циклы и node-signed подмена снимка запрещены | Rust `signed_organization_tree_replication_rejects_snapshot_rewrite`, API cycle test, integrity audit | Проверено |
| Подписанный акт инвентаризации | завершённая сессия выгружается как portable JSON с каноническими байтами, SHA-256, Ed25519 node-proof и ссылками на device-signed records; принимающая нода различает trusted/untrusted key, чужую организацию, missing/mismatched history; standalone CLI проверяет файл без БД/сети | Rust проверяет байты/hash/signature, подмену, неизвестный ключ, отсутствующую историю и tenant isolation; Playwright скачивает и проверяет оригинал/подделку; `npm run smoke` передаёт реальный акт отдельному CLI-процессу и проверяет exit `0/2` | Проверено |
| Справочники после offline-разрыва | склады, площадки, категории, бренды и статусы — device-signed portable branches с GUID/tombstone; карточки переносят ссылки по GUID | Rust `signed_config_survives_offline_sync_and_rejects_falsification`, integrity audit | Проверено |
| 100 узлов | разреженная топология, 100 процессов/БД, обязательная QR V2 выдача на leaf, отказ и cold restart 10 процессов; журнал и CAS догоняются после восстановления | `npm run scale:test`: 100/100 за 19,8 с, leaf→root за 2,1 с, peer limit 4, RSS p95 10,8 MiB | Проверено 2026-09-01 |
| Фото и документы | браузерная загрузка до 20 МБ (base64 остаётся внутри request cap 32 МБ), CAS/mesh до 32 МБ; карточка выбирает ACL, CreateTool скрывает документы без `manageDocuments` и не маскирует частичный сбой общим success. Scoped CAS ingest требует `itemId`/организацию/право, а signed V3 intent связывает GUID, hash, MIME, автора и ACL; chunks, resume, metadata/smart/full | Playwright загружает PDF из создания и accounting PDF из карточки, проверяет CAS/ACL/Ledger V3; `npm run sync:test` восстанавливает offline bytes/ACL; Rust проверяет manager-without-create, rollback и trusted-node rewrite | Проверено для новых вложений; старые записи явно legacy |
| Списание с фото | двухфазный scoped CAS ingest с одноразовым user/workspace/item/operation grant; атомарное погашение, количественная проводка и Ledger V3 commitment; portable import сверяет signed intent, UUID, количество и CAS | Rust wrong-operation/replay/required-photo, compact-intent и three-DB trusted-node rewrite tests; `npm run capability:test` переносит запись и байты на две отдельные full-ноды без межорганизационной утечки | Проверено для новых списаний; старые записи явно legacy |
| Любой узел → full | смена content mode и фоновая догрузка | `npm run sync:test` | Проверено |
| Чат и wiki | V3 user intent для текста/ACL/parent/CAS attachments; файлы загружаются двухфазно. Wiki использует часовые одноразовые grants, связанные с user/workspace/revision/hash, проверяет фактический MIME и атомарно погашает весь набор; старые записи explicit legacy | Playwright browser uploads, `npm run mesh:test`, `npm run sync:test` с offline attachments, Rust wrong-revision/MIME/replay и trusted-node rewrite tests | Проверено для новых сообщений и ревизий |
| Bit и бухгалтерия | кошелёк, прямой платёж и portable двухфазная продажа штучных ТМЦ и части количественной партии: seller offer → partition sync → buyer accept/reject → обратная сходимость Bit + custody | Playwright формирует предложение 4 из 6 выданных единиц и проверяет отсутствие раннего списания/custody; Rust проверяет partial holdings 6→2+4, две независимые БД, delta round-trip, балансы/ответственного, подмену цены/количества, replay, insufficient funds, stale seller и seller spoof | Проверено |
| Android | тот же Rust backend, Keystore, private UI/sync-only LAN | APK verifier, `npm run mobile:node:test` | Проверено ARM64/x86_64 |
| Смена Wi‑Fi/hotspot | NetworkCallback → JNI, динамический HMAC-анонс | Android contract, `npm run discovery:test` | Проверено на host/JNI build |
| SQLCipher | отдельный feature, обязательный ключ, wrong-key rejection | `npm run encrypted-db:test` | Проверено Linux |
| Backup/restore | online `.backup`, шифрование, integrity-check, новый target | `npm run backup:restore:test` | Проверено Linux |
| Потоковый transport core | MTU frames, out-of-order, duplicate, missing ranges, SHA-256, Android JNI и безключевой `meshkeeper-frame` stdin/stdout bridge для serial/LoRa/USB | Rust `stream_transport::tests`, encrypted bundle round-trip, APK symbols; `npm run transport:test` запускает отдельные CLI-процессы, переставляет/теряет/дублирует/портит кадры | Проверено |
| Android BLE GATT | foreground advertiser/server + scanner/client, MTU/retry, bounded crash-safe ciphertext spool, authorized import handoff, persistent diagnostics | Rust API regression, Android compile/lint, API 34 emulator instrumentation gate, APK | Реализовано; KVM CI gate настроен, но ещё не зафиксирован внешний успешный run; RF-тест на двух телефонах не выполнен |

## Release gates

Быстрый обязательный gate:

```bash
npm run production:audit
```

Единый локальный gate кандидата в релиз запускается командой:

```bash
npm run release:candidate:audit
```

Она последовательно выполняет `production:audit`, тяжёлый `scale:test`, заново
собирает PWA и Android debug APK с текущим Rust backend для ARM64/x86_64, а затем
проверяет содержимое APK и JNI-symbols. Поэтому ранее собранный APK не может
случайно послужить доказательством новой версии. Для публичного Android-релиза
после этого отдельно собирается подписанный release APK: debug-подпись не
заменяет секретный keystore владельца приложения.

Последний полный локальный backend/PWA gate 2026-09-01: `production:audit` и
`scale:test` завершились без ошибок. Масштабный прогон поднял 100 отдельных
процессов с отдельными БД, распространил подписанные чат/выдачу и CAS-вложение,
остановил и перезапустил 10 узлов, после чего все 10 догнали журнал и файл.

## Не закрыто и поэтому не заявляется production-ready

- Android BLE GATT adapter и foreground lifecycle реализованы, но ещё не
  проверены между двумя физическими телефонами с убийством Activity/процесса.
  Bluetooth Mesh managed flooding и конкретный LoRa radio driver
  пока не интегрированы. Общий bounded MTU framing, out-of-order сборка, resume
  и production CLI pipe/serial bridge готовы и тестируются; без IP также работает
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
