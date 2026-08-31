# Транспортно-независимая синхронизация Everyday

Этот документ описывает фактически реализованный протокол Rust-узла. Ledger не
считает HTTP, Bluetooth, файл или relay источником доверия: транспорт доставляет
байты, а принимающая нода проверяет их до изменения SQLite.

## Криптографическая единица обмена

Узел экспортирует UTF-8 JSON journal версии 1. В него входят:

- `history` — account-chain события организаций;
- актуальные проекции инвентаря, участников, структуры и чата;
- двойная бухгалтерская запись Bit;
- revision DAG базы знаний;
- `contentCatalog` и `contentProviders`, но не бинарные файлы;
- `frontier` для инкрементального обмена;
- `journalHash`, `journalPublicKey`, `journalSignature`.

Хэш journal — SHA-256 от доменно разделённого канонического JSON без трёх полей
подписи. Подпись — Ed25519. События ledger также имеют SHA-256, Ed25519-подпись
ноды и ссылку `prevHash` в цепочке `(workspace, public key)`. Критические
пользовательские операции дополнительно содержат подписанный device request:
точный HTTP path, SHA-256 тела, nonce и timestamp.

До commit принимающая сторона проверяет journal hash/signature, registry ключей
нод, ledger chains, ссылки чата, бухгалтерский баланс и хэши wiki-ревизий.
Ошибка откатывает весь импорт до savepoint. Повторы безопасны благодаря GUID,
хэшам и детерминированной reconciliation.

## HTTP/LAN transport

Реализованные endpoints:

- `GET /sync/hello` — возможности и роль узла;
- `GET /sync/journal` — полный подписанный snapshot;
- `POST /sync/journal/pull` — delta относительно переданного frontier;
- `POST /sync/journal` — проверка и применение journal;
- `GET /sync/blob/{hash}?offset=N` — CAS-фрагмент; bearer для запроса
  детерминированно выводится HMAC от общего transport token и одного hash.

Transport закрыт bearer-токеном не короче 32 символов. Токен разрешает соединение,
но не позволяет подделать journal. В production нужен HTTPS; незашифрованный HTTP
разрешается только явной настройкой для изолированной LAN.

Узел периодически обходит настроенные peers и upstream. Потеря связи не блокирует
локальные операции. При восстановлении peer сначала отдаёт delta по frontier,
затем получает встречный delta. Старый полный snapshot не откатывает более новую
историю.

### Самоорганизация offline LAN

При заданном `MESHKEEPER_DISCOVERY_BIND` узел каждые пять секунд отправляет UDP-
анонс `everyday/lan-discovery/v1`. В нём находятся node ID, локальный sync URL,
Unix-время, случайный nonce и HMAC-SHA256 общего transport token. Получатель:

1. ограничивает datagram 2048 байтами и требует точную JSON-схему;
2. проверяет HMAC в постоянное время, окно часов ±120 секунд и одноразовый nonce;
3. отбрасывает собственный node ID;
4. принимает только IP literal из private/link-local/loopback диапазона с портом;
5. добавляет endpoint в ограниченный список peers и немедленно запускает sync.

Обнаружение не обходит trust registry: новый Ed25519 key после соединения всё
равно попадает в pending и требует одобрения владельца. Android использует UDP
broadcast `255.255.255.255:8767`; desktop-узлы включают его явно. Это работает
без Internet через одну Wi-Fi LAN или hotspot и не является BLE Mesh transport.

## Файл и системный Share

`everyday-sync-bundle` версии 2 шифрует полный подписанный journal:

```json
{
  "format": "everyday-sync-bundle",
  "version": 2,
  "createdAt": "RFC3339",
  "cipher": "XChaCha20-Poly1305",
  "kdf": "HKDF-SHA256",
  "nonce": "base64-no-pad",
  "ciphertext": "base64-no-pad"
}
```

Ключ envelope доменно отделяется от HTTP bearer и CAS capabilities через
HKDF-SHA256 с отдельными salt/info. AEAD использует случайный 192-битный nonce
и фиксированный AAD формата. Поэтому системный Share, Bluetooth-посредник или
потерянный USB-носитель не раскрывает участников, чат и бухгалтерию. После
расшифровки остаётся обязательной независимая Ed25519-проверка journal и ledger.
Экспорт без `MESHKEEPER_SYNC_TOKEN` запрещён. Legacy v1 принимается только для
миграции старых подписанных файлов, но новые plaintext-пакеты не создаются.

В «Админка → Офлайн-узлы» пакет можно передать системным Share через Bluetooth,
Wi‑Fi Direct, AirDrop, USB или произвольный store-and-forward канал. Импорт имеет
лимит 30 МБ и сам является Ed25519-подписанной операцией принимающего устройства.
URL из переносимого файла не добавляется в peers автоматически. Подмена любого
поля ciphertext обнаруживается AEAD, а подмена journal — его подписью.
CAS-бинарники в bundle не включаются.

## CAS transport

Файл адресуется как `cas:<sha256>`. Manifest содержит hash, MIME и размер.
Получатель скачивает объект фрагментами по 64 КиБ, продолжает с последнего offset
и публикует blob только после совпадения полного SHA-256. Capability раскрывает
провайдеру право только на запрошенный hash, а не общий mesh-токен.

Политика хранения задаётся независимо:

- `metadata` — journal и manifests;
- `smart` — явные pins и нужные миниатюры;
- `full` — все обнаруженные доступные blobs.

Любой узел можно переключить в `full`; metadata-узел способен gossip-передать
адрес настоящего провайдера, не сохраняя blob у себя.

## Контракт будущего потокового адаптера

BLE GATT, Bluetooth Mesh, Wi‑Fi Direct, WebRTC, LoRa или последовательный порт
должны переносить неизменённый signed journal/bundle и CAS chunks. Минимальный
адаптер обязан:

1. обменять версию протокола, node public key и frontier;
2. передать length-prefixed JSON delta с ограничением размера;
3. подтвердить journal hash после успешного атомарного импорта;
4. безопасно повторять неподтверждённые пакеты;
5. передавать CAS отдельно по hash/offset;
6. не интерпретировать и не переподписывать ledger-события.

CRC/MTU-фрагментация допустимы как защита канала от случайных ошибок, но не
заменяют SHA-256 и Ed25519. Для конфиденциальности нужен защищённый канал или
отдельное end-to-end шифрование: текущий journal подписан, но его текст не
зашифрован.

## Реальный статус платформ

- Windows, Linux и macOS: один Rust-бинарник с локальной SQLite и PWA.
- Android: foreground service через JNI запускает тот же Rust/SQLite узел;
  WebView работает с loopback UI, отдельный LAN listener публикует только sync,
  а HMAC UDP broadcast автоматически находит соседние телефоны в offline LAN.
- Обмен телефонов без IP: уже возможен вручную через системный файловый Share.
- iOS-оболочка и потоковый BLE Mesh adapter ещё не реализованы; документация не
  должна заявлять обратное. Android ABI/APK проверяются отдельным CI job.
