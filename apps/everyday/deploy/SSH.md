# Доступ по SSH-ключу для выкладки

Цель: `deploy/deploy.sh` должен ходить на сервер без ввода пароля. Тогда
выкладку можно запускать автоматически, а секрет остаётся у вас — ни скрипт,
ни ассистент его не видят и не вводят.

**Быстрый путь.** Всё из шагов 1–4 делает один скрипт:

```powershell
./deploy/bootstrap.ps1 -Server ВАШ_СЕРВЕР
```

Пароль сервера спросит `ssh` — один раз. Дальше пароль не нужен вообще.
Ниже те же шаги вручную, если хочется понимать, что происходит.

## 1. Ключ на рабочей машине

Отдельный ключ только для выкладки — не тот, которым вы ходите на другие
серверы. Windows 11, PowerShell:

```powershell
ssh-keygen -t ed25519 -f "$env:USERPROFILE\.ssh\meshkeeper_deploy" -C "meshkeeper deploy"
```

Про парольную фразу — см. раздел 6. Приватный файл
`meshkeeper_deploy` никому не передавайте; наружу уходит только
`meshkeeper_deploy.pub`.

## 2. Публичный ключ на сервер

Пользователь `meshkeeper` уже создан (см. `README.md`, шаг 1). Здесь пароль
сервера понадобится в последний раз:

```powershell
Get-Content "$env:USERPROFILE\.ssh\meshkeeper_deploy.pub" | ssh root@ВАШ_СЕРВЕР `
  "install -d -m 700 -o meshkeeper -g meshkeeper /opt/meshkeeper/.ssh && cat >> /opt/meshkeeper/.ssh/authorized_keys && chown meshkeeper:meshkeeper /opt/meshkeeper/.ssh/authorized_keys && chmod 600 /opt/meshkeeper/.ssh/authorized_keys"
```

`ssh-copy-id` в Windows нет, поэтому команда длинная — она делает то же самое.

Полезно сузить ключ прямо в `authorized_keys`: разрешить вход только с
вашего адреса и запретить проброс портов. Допишите перед `ssh-ed25519`:

```
from="ВАШ.IP.АДРЕС",no-agent-forwarding,no-port-forwarding,no-X11-forwarding ssh-ed25519 AAAA...
```

## 3. Короткое имя хоста

`~/.ssh/config` на рабочей машине:

```
Host meshkeeper
    HostName ВАШ_СЕРВЕР
    User meshkeeper
    IdentityFile ~/.ssh/meshkeeper_deploy
    IdentitiesOnly yes
```

`IdentitiesOnly yes` важен: иначе клиент переберёт все ваши ключи и сервер
может отбить попытку по лимиту.

## 4. Первое подключение — вручную

```powershell
ssh meshkeeper
```

При первом заходе SSH покажет отпечаток сервера и спросит подтверждение.
**Сделайте это сами и сверьте отпечаток** — так он попадёт в `known_hosts`.

Это не формальность: `deploy.sh` ходит с `BatchMode=yes`, и если отпечаток
неизвестен, выкладка просто упадёт с ошибкой проверки хоста, а не спросит.

## 5. Проверка

Должно отработать молча, без единого приглашения:

```powershell
ssh -o BatchMode=yes meshkeeper "echo ok && test -d /opt/meshkeeper && echo dirs-ok"
```

Дальше выкладка:

```powershell
$env:MESHKEEPER_DEPLOY_HOST='meshkeeper'
$env:MESHKEEPER_DEPLOY_USER='meshkeeper'
./deploy/deploy.sh
```

## 6. Парольная фраза ключа

Два варианта, оба рабочие:

**С парольной фразой (безопаснее).** Ключ на диске бесполезен без фразы, но
для запуска без вопросов её нужно один раз за сессию отдать агенту:

```powershell
Start-Service ssh-agent
Set-Service ssh-agent -StartupType Automatic
ssh-add "$env:USERPROFILE\.ssh\meshkeeper_deploy"
```

Оговорка: агент Windows видит только `ssh.exe` из состава Windows. Git Bash
приносит свой `ssh` и к этому агенту не подключается, поэтому выкладку
запускайте из PowerShell.

**Без парольной фразы (проще).** Файл ключа сам по себе даёт доступ — это
постоянный секрет на вашем диске. Приемлемо, потому что ключ ведёт под
непривилегированного пользователя, `sudo` у него ограничен белым списком из
`/etc/sudoers.d/meshkeeper`, а `from=` в `authorized_keys` привязывает его к
вашему адресу. Root-доступа этот ключ не даёт.

## 7. Закрыть парольный вход

После того как ключ заработал:

```bash
sudo sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config
sudo sed -i 's/^#\?PermitRootLogin.*/PermitRootLogin prohibit-password/' /etc/ssh/sshd_config
sudo sshd -t && sudo systemctl reload ssh
```

Не закрывайте текущую SSH-сессию, пока не проверите вход в новом окне —
иначе при ошибке в конфиге можно потерять доступ к серверу.

## Что делает ассистент, а что нет

- **Делает:** запускает `deploy/deploy.sh`, читает вывод, чинит падения
  выкладки. Аутентификацией занимается сам SSH — ключом или агентом.
- **Не делает:** не вводит пароли, не открывает приватный ключ, не копирует
  парольную фразу. Если команда запросит пароль, работа останавливается —
  шаг нужно выполнить вам.
