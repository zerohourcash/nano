<#
.SYNOPSIS
Готовит сервер к выкладке MeshKeeper. Запускается один раз.

.DESCRIPTION
Делает всё, для чего нужен пароль сервера, за одно подключение:
  * создаёт ключ для выкладки на этой машине (если его ещё нет);
  * заводит на сервере пользователя meshkeeper и его каталоги;
  * кладёт публичный ключ в authorized_keys;
  * генерирует общий секрет синхронизации;
  * ставит /usr/local/sbin/meshkeeper-activate и узкое правило sudo;
  * прописывает алиас хоста в ~/.ssh/config.

Пароль сервера спросит сам ssh — введёте его один раз, вручную.
Скрипт пароль не сохраняет и никуда не передаёт.

.EXAMPLE
./deploy/bootstrap.ps1 -Server 203.0.113.10
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Server,

    # Учётка с правами root для первичной настройки.
    [string]$AdminUser = 'root',

    # Имя записи в ~/.ssh/config, его потом передаём в deploy.sh.
    [string]$Alias = 'meshkeeper'
)

$ErrorActionPreference = 'Stop'

$sshDir = Join-Path $env:USERPROFILE '.ssh'
$keyPath = Join-Path $sshDir 'meshkeeper_deploy'
$pubPath = "$keyPath.pub"

if (-not (Test-Path $sshDir)) {
    New-Item -ItemType Directory -Path $sshDir | Out-Null
}

# ── 1. Ключ выкладки ────────────────────────────────────────────────────────
if (Test-Path $keyPath) {
    Write-Host "Ключ уже есть: $keyPath" -ForegroundColor DarkGray
}
else {
    Write-Host 'Создаю ключ для выкладки' -ForegroundColor Cyan
    # Без парольной фразы: ключ ведёт под непривилегированного пользователя,
    # sudo у него ограничен одной командой. Иначе выкладка требовала бы
    # разблокировки агента при каждом запуске.
    ssh-keygen -t ed25519 -f $keyPath -N '""' -C 'meshkeeper deploy' | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'ssh-keygen не отработал' }
}

$pubKey = (Get-Content $pubPath -Raw).Trim()

# ── 2. Скрипт настройки сервера ─────────────────────────────────────────────
# Одинарные кавычки здесь принципиальны: внутри полно $-конструкций bash,
# и любая интерполяция PowerShell выполнила бы их на этой машине.
$remoteTemplate = @'
set -euo pipefail

PUBKEY='__PUBKEY__'

if ! id -u meshkeeper >/dev/null 2>&1; then
  adduser --disabled-password --gecos "" --home /opt/meshkeeper --shell /bin/bash meshkeeper
fi

mkdir -p /opt/meshkeeper /var/lib/meshkeeper /etc/meshkeeper
chown -R meshkeeper:meshkeeper /opt/meshkeeper /var/lib/meshkeeper

install -d -m 700 -o meshkeeper -g meshkeeper /opt/meshkeeper/.ssh
touch /opt/meshkeeper/.ssh/authorized_keys
grep -qxF "$PUBKEY" /opt/meshkeeper/.ssh/authorized_keys \
  || echo "$PUBKEY" >> /opt/meshkeeper/.ssh/authorized_keys
chown meshkeeper:meshkeeper /opt/meshkeeper/.ssh/authorized_keys
chmod 600 /opt/meshkeeper/.ssh/authorized_keys

# Общий секрет сервера и локальных узлов. Существующий не перегенерируем:
# иначе отвалятся уже настроенные узлы.
if [ ! -f /etc/meshkeeper/meshkeeper.env ]; then
  printf 'MESHKEEPER_SYNC_TOKEN=%s\n' "$(openssl rand -hex 32)" > /etc/meshkeeper/meshkeeper.env
  chmod 600 /etc/meshkeeper/meshkeeper.env
fi

# Единственная команда, которую выкладке разрешено выполнять от root.
cat > /usr/local/sbin/meshkeeper-activate <<'ACTIVATE'
#!/usr/bin/env bash
# Переключает сервис на версию, загруженную в /opt/meshkeeper/incoming.
set -euo pipefail
cd /opt/meshkeeper
[ -x incoming/meshkeeper-node ] || { echo 'нет incoming/meshkeeper-node' >&2; exit 1; }
[ -f incoming/meshkeeper.service ] || { echo 'нет incoming/meshkeeper.service' >&2; exit 1; }

install -m 0644 incoming/meshkeeper.service /etc/systemd/system/meshkeeper.service
systemctl daemon-reload
systemctl stop meshkeeper 2>/dev/null || true

install -m 0755 -o meshkeeper -g meshkeeper incoming/meshkeeper-node ./meshkeeper-node
rm -rf ./public.old
[ -d ./public ] && mv ./public ./public.old
mv incoming/public ./public
chown -R meshkeeper:meshkeeper ./public
rm -rf incoming ./public.old

systemctl enable meshkeeper >/dev/null 2>&1 || true
systemctl restart meshkeeper
sleep 2
systemctl is-active --quiet meshkeeper || { journalctl -u meshkeeper -n 40 --no-pager >&2; exit 1; }
echo ACTIVATE_OK
ACTIVATE
chmod 0755 /usr/local/sbin/meshkeeper-activate

printf 'meshkeeper ALL=(root) NOPASSWD: /usr/local/sbin/meshkeeper-activate\n' \
  > /etc/sudoers.d/meshkeeper
chmod 0440 /etc/sudoers.d/meshkeeper
visudo -c -f /etc/sudoers.d/meshkeeper >/dev/null

command -v cargo >/dev/null 2>&1 || echo 'ЗАМЕТКА: cargo не установлен — Linux-бинарник придётся собирать не здесь'
command -v nginx >/dev/null 2>&1 || echo 'ЗАМЕТКА: nginx не установлен'

echo BOOTSTRAP_OK
'@

$remote = $remoteTemplate.Replace('__PUBKEY__', $pubKey).Replace("`r`n", "`n")

# base64 избавляет от возни с экранированием кавычек между PowerShell и bash.
$bytes = [Text.Encoding]::UTF8.GetBytes($remote)
$encoded = [Convert]::ToBase64String($bytes)

Write-Host ''
Write-Host 'Подключаюсь к серверу. Сейчас ssh спросит пароль — введите его.' -ForegroundColor Cyan
Write-Host 'Это единственный раз, когда пароль понадобится.' -ForegroundColor DarkGray
Write-Host ''

$output = ssh -o StrictHostKeyChecking=accept-new "$AdminUser@$Server" "echo $encoded | base64 -d | bash"
$sshExit = $LASTEXITCODE
$output | ForEach-Object { Write-Host "   $_" }

if ($output -notcontains 'BOOTSTRAP_OK') {
    # 255 — ssh не смог установить соединение (в том числе смена ключа хоста).
    if ($sshExit -eq 255) {
        $known = Join-Path $sshDir 'known_hosts'
        $seen = $null
        if (Test-Path $known) { $seen = ssh-keygen -F $Server -f $known 2>$null }

        Write-Host ''
        if ($seen) {
            Write-Host 'ОТПЕЧАТОК СЕРВЕРА НЕ СОВПАЛ С ЗАПИСАННЫМ.' -ForegroundColor Red
            Write-Host ''
            Write-Host 'Это бывает по двум причинам:' -ForegroundColor Yellow
            Write-Host '  * сервер переустановили или IP выдали другой машине — тогда всё в порядке;'
            Write-Host '  * трафик перехватывают либо сервером завладел кто-то ещё.'
            Write-Host ''
            Write-Host 'Сверьте новый отпечаток через панель хостера (VNC/консоль):' -ForegroundColor Yellow
            Write-Host '  ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub'
            Write-Host ''
            Write-Host 'Совпал — удалите устаревшую запись и повторите:' -ForegroundColor Yellow
            Write-Host "  ssh-keygen -R $Server"
            Write-Host ''
            Write-Host 'Не совпал — не подключайтесь и разбирайтесь с хостером.' -ForegroundColor Red
        }
        else {
            Write-Host 'Не удалось подключиться к серверу.' -ForegroundColor Red
            Write-Host 'Проверьте адрес, доступность порта 22 и правильность пароля.'
        }
        Write-Host ''
    }
    throw 'Настройка сервера не завершилась. Смотрите вывод выше.'
}

# ── 3. Алиас хоста ──────────────────────────────────────────────────────────
$configPath = Join-Path $sshDir 'config'
$identity = $keyPath -replace '\\', '/'
$entry = @"

Host $Alias
    HostName $Server
    User meshkeeper
    IdentityFile $identity
    IdentitiesOnly yes
"@

$existing = if (Test-Path $configPath) { Get-Content $configPath -Raw } else { '' }
if ($existing -match "(?m)^Host\s+$([regex]::Escape($Alias))\s*$") {
    Write-Host "Запись Host $Alias в ~/.ssh/config уже есть" -ForegroundColor DarkGray
}
else {
    # ascii, а не utf8: PowerShell 5.1 добавил бы BOM, и ssh не понял бы файл.
    Add-Content -Path $configPath -Value $entry -Encoding ascii
    Write-Host "Добавил Host $Alias в ~/.ssh/config" -ForegroundColor Cyan
}

# ── 4. Проверка входа по ключу ──────────────────────────────────────────────
Write-Host ''
Write-Host 'Проверяю вход по ключу (пароль спрашивать не должен)' -ForegroundColor Cyan
$probe = ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new $Alias 'echo KEY_OK'
if ($probe -notcontains 'KEY_OK') {
    throw "Вход по ключу не работает. Проверьте: ssh -v $Alias"
}

Write-Host ''
Write-Host 'Готово. Вход по ключу работает.' -ForegroundColor Green
Write-Host ''
Write-Host 'Дальше:' -ForegroundColor Green
Write-Host "  1. Напишите ассистенту: сервер готов, алиас $Alias"
Write-Host '  2. Смените root-пароль сервера — старый скомпрометирован.'
Write-Host '  3. Для HTTPS нужен домен, направленный на этот сервер:'
Write-Host '     без TLS вход в приложение работать не будет (cookie Secure).'
