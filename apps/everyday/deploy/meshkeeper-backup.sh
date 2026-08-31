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
#   MESHKEEPER_BACKUP_PASS    пароль шифрования; без него копия остаётся открытой

set -euo pipefail

DB="${MESHKEEPER_DB:-/var/lib/meshkeeper/meshkeeper.db}"
DEST="${MESHKEEPER_BACKUP_DIR:-/var/backups/meshkeeper}"
KEEP="${MESHKEEPER_BACKUP_KEEP:-14}"
STAMP="$(date +%Y%m%d-%H%M%S)"

if [ ! -f "$DB" ]; then
  echo "базы нет: $DB" >&2
  exit 1
fi

mkdir -p "$DEST"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# .backup корректно работает на живой базе в режиме WAL, в отличие от cp.
if command -v sqlite3 >/dev/null 2>&1; then
  sqlite3 "$DB" ".backup '$TMP/meshkeeper.db'"
else
  echo "нет sqlite3, копирую файлы базы целиком" >&2
  cp "$DB" "$TMP/meshkeeper.db"
  [ -f "$DB-wal" ] && cp "$DB-wal" "$TMP/meshkeeper.db-wal"
  [ -f "$DB-shm" ] && cp "$DB-shm" "$TMP/meshkeeper.db-shm"
fi

gzip -9 "$TMP/meshkeeper.db"
OUT="$DEST/meshkeeper-$STAMP.db.gz"

if [ -n "${MESHKEEPER_BACKUP_PASS:-}" ] && command -v openssl >/dev/null 2>&1; then
  # Симметричное шифрование с выводом ключа из пароля: копию можно класть
  # в облако, не раскрывая содержимое инвентаризации.
  openssl enc -aes-256-cbc -pbkdf2 -iter 200000 -salt \
    -in "$TMP/meshkeeper.db.gz" -out "$OUT.enc" \
    -pass env:MESHKEEPER_BACKUP_PASS
  OUT="$OUT.enc"
else
  cp "$TMP/meshkeeper.db.gz" "$OUT"
  echo "ВНИМАНИЕ: MESHKEEPER_BACKUP_PASS не задан, копия не зашифрована" >&2
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
