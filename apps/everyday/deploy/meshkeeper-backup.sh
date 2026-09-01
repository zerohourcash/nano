#!/usr/bin/env bash
# Резервная копия базы MeshKeeper.
#
# ТЗ §6 требует обязательную резервную копию. В серверной схеме её делает сам
# сервер: копия снимается штатным механизмом SQLite (без остановки сервиса),
# затем сжимается и шифруется, старые копии удаляются по сроку хранения.
#
# Запускается таймером systemd, см. meshkeeper-backup.timer.
#
# Переменные (из /etc/meshkeeper/meshkeeper.env):
#   MESHKEEPER_DB             путь к базе
#   MESHKEEPER_BACKUP_DIR     куда складывать (по умолчанию /var/backups/meshkeeper)
#   MESHKEEPER_BACKUP_KEEP    сколько копий хранить (по умолчанию 14)
#   MESHKEEPER_BACKUP_PASS    обязательный пароль шифрования

set -euo pipefail
umask 077

DB="${MESHKEEPER_DB:-/var/lib/meshkeeper/meshkeeper.db}"
DEST="${MESHKEEPER_BACKUP_DIR:-/var/backups/meshkeeper}"
KEEP="${MESHKEEPER_BACKUP_KEEP:-14}"
STAMP="$(date +%Y%m%d-%H%M%S)-$$"

if ! [[ "$KEEP" =~ ^[0-9]+$ ]] || [ "$KEEP" -lt 1 ] || [ "$KEEP" -gt 3650 ]; then
  echo "MESHKEEPER_BACKUP_KEEP должен быть целым числом 1..3650" >&2
  exit 1
fi

if [ ! -f "$DB" ]; then
  echo "базы нет: $DB" >&2
  exit 1
fi

mkdir -p "$DEST"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# .backup корректно работает на живой базе в режиме WAL, в отличие от cp.
command -v sqlite3 >/dev/null 2>&1 || {
  echo "для согласованной online-копии требуется sqlite3" >&2
  exit 1
}
sqlite3 "$DB" ".backup '$TMP/meshkeeper.db'"
sqlite3 "$TMP/meshkeeper.db" "PRAGMA integrity_check" | grep -qx 'ok' || {
  echo "SQLite integrity_check резервной копии не пройден" >&2
  exit 1
}

gzip -9 "$TMP/meshkeeper.db"
OUT="$DEST/meshkeeper-$STAMP.db.gz"

if [ -z "${MESHKEEPER_BACKUP_PASS:-}" ]; then
  echo "MESHKEEPER_BACKUP_PASS обязателен: plaintext backup запрещён" >&2
  exit 1
fi
command -v openssl >/dev/null 2>&1 || {
  echo "для шифрования резервной копии требуется openssl" >&2
  exit 1
}
if [ -n "${MESHKEEPER_BACKUP_PASS:-}" ]; then
  # Симметричное шифрование с выводом ключа из пароля: копию можно класть
  # в облако, не раскрывая содержимое инвентаризации.
  openssl enc -aes-256-cbc -pbkdf2 -iter 200000 -salt \
    -in "$TMP/meshkeeper.db.gz" -out "$OUT.enc" \
    -pass env:MESHKEEPER_BACKUP_PASS
  OUT="$OUT.enc"
fi

chmod 600 "$OUT"
echo "копия готова: $OUT ($(du -h "$OUT" | cut -f1))"

# Ротация по количеству копий.
mapfile -t OLD < <(ls -1t "$DEST"/meshkeeper-*.db.gz* 2>/dev/null | tail -n +"$((KEEP + 1))")
for f in "${OLD[@]:-}"; do
  [ -n "$f" ] || continue
  rm -f "$f"
  echo "удалена старая копия: $(basename "$f")"
done

echo "всего копий: $(ls -1 "$DEST"/meshkeeper-*.db.gz* 2>/dev/null | wc -l)"
