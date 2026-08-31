<#
.SYNOPSIS
Настраивает HTTPS и автоматические резервные копии. Запускается один раз, нужен root.

.DESCRIPTION
Без TLS вход в приложение не работает: сессия живёт в cookie с флагом Secure.
Скрипт ставит certbot, выпускает сертификат Let's Encrypt, настраивает nginx,
а также ставит ежедневное резервное копирование базы (ТЗ §6) с шифрованием
и ротацией.

Домен покупать не нужно: sslip.io отдаёт адрес вида
203-0-113-10.sslip.io, который резолвится в тот же IP, и Let's Encrypt
выдаёт на него обычный доверенный сертификат.

Пароль root спросит сам ssh — один раз.

.EXAMPLE
./deploy/tls-setup.ps1 -Server 203.0.113.10

.EXAMPLE
./deploy/tls-setup.ps1 -Server 203.0.113.10 -Domain meshkeeper.mycompany.ru -Email me@mycompany.ru
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Server,

    # Свой домен, если есть. По умолчанию — обёртка sslip.io над IP.
    [string]$Domain = '',

    # Почта для уведомлений об истечении сертификата. Пусто — регистрация без почты.
    [string]$Email = '',

    [string]$AdminUser = 'root'
)

$ErrorActionPreference = 'Stop'

if (-not $Domain) {
    $Domain = ($Server -replace '\.', '-') + '.sslip.io'
    Write-Host "Домен не задан, использую $Domain" -ForegroundColor DarkGray
}

$emailArg = if ($Email) { "--email $Email" } else { '--register-unsafely-without-email' }

# Скрипт и юниты резервного копирования уезжают в том же подключении,
# чтобы пароль root спрашивался ровно один раз.
$deployDir = Split-Path -Parent $PSCommandPath
function Encode-File([string]$name) {
    $path = Join-Path $deployDir $name
    if (-not (Test-Path $path)) { throw "Не найден $path" }
    $text = (Get-Content $path -Raw) -replace "`r`n", "`n"
    [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($text))
}
$backupSh = Encode-File 'meshkeeper-backup.sh'
$backupService = Encode-File 'meshkeeper-backup.service'
$backupTimer = Encode-File 'meshkeeper-backup.timer'

# Одинарные кавычки: внутри bash-переменные, PowerShell не должен их трогать.
$template = @'
set -euo pipefail

DOMAIN='__DOMAIN__'
EMAIL_ARG='__EMAIL_ARG__'

export DEBIAN_FRONTEND=noninteractive

if ! command -v certbot >/dev/null 2>&1; then
  echo "ставлю certbot"
  apt-get update -qq
  apt-get install -y -qq certbot python3-certbot-nginx >/dev/null
fi

command -v sqlite3 >/dev/null 2>&1 || apt-get install -y -qq sqlite3 >/dev/null || true

mkdir -p /var/lib/meshkeeper /etc/meshkeeper
chown meshkeeper:meshkeeper /var/lib/meshkeeper

# Временный конфиг: certbot должен пройти проверку по HTTP.
cat > /etc/nginx/sites-available/meshkeeper <<NGINX
server {
    listen 80 default_server;
    listen [::]:80 default_server;
    server_name $DOMAIN;

    location /.well-known/acme-challenge/ { root /var/www/html; }
    location / { return 301 https://\$host\$request_uri; }
}
NGINX

rm -f /etc/nginx/sites-enabled/default
ln -sf /etc/nginx/sites-available/meshkeeper /etc/nginx/sites-enabled/meshkeeper
mkdir -p /var/www/html
nginx -t >/dev/null
systemctl reload nginx

echo "выпускаю сертификат для $DOMAIN"
certbot certonly --webroot -w /var/www/html -d "$DOMAIN" \
  --non-interactive --agree-tos $EMAIL_ARG --keep-until-expiring

# Боевой конфиг с TLS.
cat > /etc/nginx/sites-available/meshkeeper <<NGINX
server {
    listen 80 default_server;
    listen [::]:80 default_server;
    server_name $DOMAIN;
    location /.well-known/acme-challenge/ { root /var/www/html; }
    location / { return 301 https://\$host\$request_uri; }
}

server {
    listen 443 ssl default_server;
    listen [::]:443 ssl default_server;
    http2 on;
    server_name $DOMAIN;

    ssl_certificate     /etc/letsencrypt/live/$DOMAIN/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/$DOMAIN/privkey.pem;
    ssl_protocols TLSv1.2 TLSv1.3;

    add_header Strict-Transport-Security "max-age=31536000" always;
    add_header X-Content-Type-Options "nosniff" always;
    add_header Referrer-Policy "same-origin" always;

    # Фотографии предметов уходят в базу как data-URL — запросы крупные.
    client_max_body_size 25m;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        # Host не подменяем: узел сверяет с ним Origin, иначе все изменяющие
        # запросы получат 403.
        proxy_set_header Host              \$host;
        proxy_set_header X-Real-IP         \$remote_addr;
        proxy_set_header X-Forwarded-For   \$proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto \$scheme;
        proxy_read_timeout 60s;
    }
}
NGINX

nginx -t >/dev/null
systemctl reload nginx
systemctl enable --now certbot.timer >/dev/null 2>&1 || true

# ── Резервное копирование ───────────────────────────────────────────────────
echo '__B_SH__'     | base64 -d > /usr/local/sbin/meshkeeper-backup
echo '__B_SERVICE__' | base64 -d > /etc/systemd/system/meshkeeper-backup.service
echo '__B_TIMER__'   | base64 -d > /etc/systemd/system/meshkeeper-backup.timer
chmod 0755 /usr/local/sbin/meshkeeper-backup
chmod 0644 /etc/systemd/system/meshkeeper-backup.service /etc/systemd/system/meshkeeper-backup.timer

# Пароль архивов создаём один раз и больше не трогаем: иначе старые копии
# станут нечитаемыми.
if ! grep -q '^MESHKEEPER_BACKUP_PASS=' /etc/meshkeeper/meshkeeper.env 2>/dev/null; then
  printf 'MESHKEEPER_BACKUP_PASS=%s\n' "$(openssl rand -hex 24)" >> /etc/meshkeeper/meshkeeper.env
fi
chmod 600 /etc/meshkeeper/meshkeeper.env
mkdir -p /var/backups/meshkeeper
chmod 700 /var/backups/meshkeeper

systemctl daemon-reload
systemctl enable --now meshkeeper-backup.timer >/dev/null

# Пробный прогон: копия должна получиться сразу, а не «когда-нибудь ночью».
systemctl start meshkeeper-backup.service
sleep 2
ls -la /var/backups/meshkeeper/ | tail -3

echo "DOMAIN=$DOMAIN"
echo TLS_OK
'@

$remote = $template.
    Replace('__DOMAIN__', $Domain).
    Replace('__EMAIL_ARG__', $emailArg).
    Replace('__B_SH__', $backupSh).
    Replace('__B_SERVICE__', $backupService).
    Replace('__B_TIMER__', $backupTimer).
    Replace("`r`n", "`n")
$encoded = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($remote))

Write-Host ''
Write-Host 'Подключаюсь как root. ssh спросит пароль — введите его.' -ForegroundColor Cyan
Write-Host ''

$output = ssh -o StrictHostKeyChecking=accept-new "$AdminUser@$Server" "echo $encoded | base64 -d | bash"
$output | ForEach-Object { Write-Host "   $_" }

if ($output -notcontains 'TLS_OK') {
    throw 'Настроить TLS не удалось. Смотрите вывод выше.'
}

Write-Host ''
Write-Host "Готово. HTTPS работает: https://$Domain" -ForegroundColor Green
Write-Host 'Резервное копирование: ежедневно, копии в /var/backups/meshkeeper' -ForegroundColor Green
Write-Host 'Напишите ассистенту: TLS готов' -ForegroundColor Green
