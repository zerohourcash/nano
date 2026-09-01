#!/usr/bin/env bash
# Проверенное восстановление зашифрованной server backup в новый файл.
set -euo pipefail
umask 077

SOURCE="${1:-}"
TARGET="${2:-}"
if [ -z "$SOURCE" ] || [ -z "$TARGET" ]; then
  echo "usage: meshkeeper-restore.sh BACKUP.db.gz.enc NEW_DATABASE.db" >&2
  exit 2
fi
if [ ! -f "$SOURCE" ]; then
  echo "копия не найдена: $SOURCE" >&2
  exit 1
fi
if [ -e "$TARGET" ]; then
  echo "целевой файл уже существует: $TARGET" >&2
  exit 1
fi
if [ -z "${MESHKEEPER_BACKUP_PASS:-}" ]; then
  echo "MESHKEEPER_BACKUP_PASS обязателен" >&2
  exit 1
fi
command -v openssl >/dev/null 2>&1 || { echo "требуется openssl" >&2; exit 1; }
command -v sqlite3 >/dev/null 2>&1 || { echo "требуется sqlite3" >&2; exit 1; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
openssl enc -d -aes-256-cbc -pbkdf2 -iter 200000 \
  -in "$SOURCE" -out "$TMP/meshkeeper.db.gz" -pass env:MESHKEEPER_BACKUP_PASS
gzip -dc "$TMP/meshkeeper.db.gz" > "$TMP/meshkeeper.db"
sqlite3 "$TMP/meshkeeper.db" "PRAGMA integrity_check" | grep -qx 'ok' || {
  echo "восстановленная SQLite не прошла integrity_check" >&2
  exit 1
}
TARGET_DIR="$(dirname "$TARGET")"
if [ ! -d "$TARGET_DIR" ]; then
  echo "целевой каталог не существует: $TARGET_DIR" >&2
  exit 1
fi
TARGET_OWNER="$(stat -c '%U' "$TARGET_DIR")"
TARGET_GROUP="$(stat -c '%G' "$TARGET_DIR")"
install -m 0600 -o "$TARGET_OWNER" -g "$TARGET_GROUP" "$TMP/meshkeeper.db" "$TARGET"
echo "восстановлено и проверено: $TARGET"
