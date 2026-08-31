# Развёртывание центрального сервера

Схема: один сервер хранит общую базу и раздаёт интерфейс, локальные узлы в
офисах работают на своих базах и обмениваются изменениями с сервером, когда
есть связь.

## 0. Перед началом — про безопасность хоста

Если сервер уже использовался с реквизитами, которые лежали в открытом виде
(см. раздел про инцидент в корневом `README.md`), считайте его
скомпрометированным. До установки:

1. смените root-пароль и SSH-ключи;
2. проверьте `~/.ssh/authorized_keys`, `systemd`-юниты, `cron`, логи входов;
3. отзовите старые приглашения и смените `MESHKEEPER_SYNC_TOKEN`.

Сборка намеренно падает, если старые реквизиты снова попадут в артефакты —
за этим следит `npm run verify:artifact`.

## 1. Подготовка сервера (одна команда)

Всё, для чего нужен пароль сервера, делается за одно подключение:

```powershell
./deploy/bootstrap.ps1 -Server ВАШ_СЕРВЕР
```

Скрипт создаст ключ выкладки, заведёт пользователя `meshkeeper` с каталогами,
положит публичный ключ, сгенерирует `MESHKEEPER_SYNC_TOKEN`, поставит
`/usr/local/sbin/meshkeeper-activate` с узким правилом sudo и пропишет алиас
хоста. Пароль спросит сам `ssh` — один раз, вручную; скрипт его не сохраняет.

После этого выкладка идёт без пароля:

```powershell
$env:MESHKEEPER_DEPLOY_HOST='meshkeeper'
$env:MESHKEEPER_DEPLOY_USER='meshkeeper'
./deploy/deploy.sh
```

Ниже — то же самое вручную, если нужен контроль над каждым шагом.

## 1б. Подготовка сервера вручную

```bash
# Пользователю нужен обычный shell: под --system обычно ставится nologin,
# и зайти по SSH для выкладки будет нельзя.
sudo adduser --disabled-password --gecos "" \
  --home /opt/meshkeeper --shell /bin/bash meshkeeper
sudo mkdir -p /opt/meshkeeper /var/lib/meshkeeper /etc/meshkeeper
sudo chown -R meshkeeper:meshkeeper /opt/meshkeeper /var/lib/meshkeeper

# Общий секрет сервера и узлов.
printf 'MESHKEEPER_SYNC_TOKEN=%s\n' "$(openssl rand -hex 32)" \
  | sudo tee /etc/meshkeeper/meshkeeper.env >/dev/null
sudo chmod 600 /etc/meshkeeper/meshkeeper.env
```

## 1a. Доступ по SSH-ключу

Пароль в деплое не участвует: аутентификация идёт ключом, и ни скрипт, ни
ассистент секрет не видят. Порядок — в `SSH.md`.

Коротко: на рабочей машине создаётся отдельный ключ, его публичная половина
кладётся в `/opt/meshkeeper/.ssh/authorized_keys` на сервере, после чего
парольный вход в SSH отключается.

Дайте пользователю право перезапускать только этот сервис:

```bash
echo 'meshkeeper ALL=(root) NOPASSWD: /bin/systemctl restart meshkeeper, /bin/systemctl stop meshkeeper, /bin/systemctl enable --now meshkeeper, /bin/systemctl daemon-reload, /usr/bin/install -m 0644 * /etc/systemd/system/meshkeeper.service' \
  | sudo tee /etc/sudoers.d/meshkeeper
```

## 2. HTTPS и резервные копии (одна команда)

```powershell
./deploy/tls-setup.ps1 -Server ВАШ_СЕРВЕР
```

Скрипт ставит certbot, выпускает сертификат Let's Encrypt, настраивает nginx
на проксирование всего трафика узлу и включает ежедневное резервное
копирование с шифрованием и ротацией.

Домен покупать не обязательно: по умолчанию берётся обёртка `sslip.io` над
IP (например `203-0-113-10.sslip.io`), она резолвится в тот же адрес, и
сертификат на неё выдаётся обычным порядком. Свой домен — параметром
`-Domain`, почта для уведомлений — `-Email`.

Ниже — то же самое вручную.

## 2б. HTTPS вручную

Узел слушает только `127.0.0.1:8080` и сам TLS не терминирует. Сессия
живёт в cookie с флагом `Secure`, поэтому **без HTTPS вход не заработает**.

```bash
sudo cp deploy/nginx-meshkeeper.conf /etc/nginx/sites-available/meshkeeper
sudo ln -s /etc/nginx/sites-available/meshkeeper /etc/nginx/sites-enabled/
sudo certbot --nginx -d meshkeeper.example.com
sudo nginx -t && sudo systemctl reload nginx
```

В конфиге замените `meshkeeper.example.com` на свой домен. Не подменяйте
заголовок `Host` в `proxy_set_header`: узел сверяет с ним `Origin`, и подмена
приведёт к отказу всех изменяющих запросов.

## 3. Сборка и выкладка

Бинарник нужен под Linux. Со стороны Windows удобнее собрать его на самом
сервере или в контейнере:

```bash
# на сервере
cargo build --release --manifest-path backend/Cargo.toml
```

Затем с рабочей машины:

```bash
npm run build                      # фронтенд + бинарник
export MESHKEEPER_DEPLOY_HOST=ВАШ_СЕРВЕР
export MESHKEEPER_DEPLOY_USER=meshkeeper
./deploy/deploy.sh
```

Скрипт не хранит и не запрашивает пароли: адрес и пользователь берутся из
окружения, аутентификация — по ключу. База в `/var/lib/meshkeeper` при
выкладке не трогается.

## 4. Первый вход

Откройте `https://meshkeeper.example.com` — на пустой базе доступна вкладка
«Создать группу». После регистрации первого владельца открытая регистрация
закрывается, остальные входят по QR-приглашению.

## 4б. Android-приложение

APK — тонкий клиент: открывает интерфейс прямо с сервера, своей копии
бэкенда больше не носит. Раньше внутри был отдельный узел на Java, который
обязан был повторять каждое изменение основного узла и неизбежно отставал.

Сборка (нужны JDK 17 и Android SDK):

```bash
npm run build                    # сначала фронтенд
cd android
gradle :app:assembleDebug        # APK в app/build/outputs/apk/debug/
```

Две особенности Windows:

* путь к проекту не должен содержать кириллицу — Gradle на таком пути
  отказывается собирать. Скопируйте проект в ASCII-каталог;
* в `local.properties` путь к SDK пишите через прямые слэши
  (`sdk.dir=C:/Users/.../Android/Sdk`). С обратными слэшами `\U` читается
  как unicode-escape, и сборка падает с «Malformed \uxxxx encoding».

Для релизной подписи нужен ваш keystore — переменные
`MESHKEEPER_ANDROID_KEYSTORE`, `..._STORE_PASSWORD`, `..._KEY_ALIAS`,
`..._KEY_PASSWORD`. Без них собирается только debug-вариант.

При первом запуске приложение спрашивает адрес сервера — введите его один
раз, дальше он сохраняется.

## 5. Локальные узлы

На каждом офисном компьютере тот же бинарник плюс две переменные:

```powershell
$env:MESHKEEPER_SYNC_TOKEN='тот же секрет, что на сервере'
$env:MESHKEEPER_UPSTREAM='https://meshkeeper.example.com'
npm start
```

Узел поднимет интерфейс локально, будет работать при пропаже интернета и
догонит сервер, когда связь вернётся. Состояние обмена — «Админка →
Офлайн-узлы».

Проверить связку целиком, не трогая production:

```bash
npm run sync:test
```
