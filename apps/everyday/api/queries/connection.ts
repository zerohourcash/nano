import fs from "node:fs";
import path from "node:path";
import Database from "better-sqlite3";
import { drizzle } from "drizzle-orm/better-sqlite3";
import { env } from "../lib/env";
import * as schema from "@db/schema";
import * as relations from "@db/relations";

const fullSchema = { ...schema, ...relations };

let sqlite: InstanceType<typeof Database> | undefined;
let instance: ReturnType<typeof drizzle<typeof fullSchema>>;

function migrateExisting(db: InstanceType<typeof Database>) {
  const cols = db.prepare("PRAGMA table_info(users)").all() as Array<{ name: string }>;
  if (!cols.some((c) => c.name === "password_hash")) {
    db.exec("ALTER TABLE users ADD COLUMN password_hash TEXT");
  }
}

function ensureSchema(db: InstanceType<typeof Database>) {
  db.exec(`
    CREATE TABLE IF NOT EXISTS workspaces (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      name TEXT NOT NULL,
      timezone TEXT NOT NULL DEFAULT 'Europe/Moscow',
      internal_id_prefix TEXT NOT NULL DEFAULT 'ВН-',
      comment TEXT,
      created_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS users (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      full_name TEXT NOT NULL,
      position TEXT,
      phone TEXT NOT NULL UNIQUE,
      avatar_url TEXT,
      status TEXT NOT NULL DEFAULT 'active',
      password_hash TEXT,
      role_rights TEXT,
      created_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS user_workspaces (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      user_id INTEGER NOT NULL,
      workspace_id INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS storages (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      name TEXT NOT NULL,
      responsible_user_id INTEGER,
      workspace_id INTEGER NOT NULL,
      address TEXT
    );
    CREATE TABLE IF NOT EXISTS building_sites (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      name TEXT NOT NULL,
      responsible_user_id INTEGER,
      workspace_id INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS categories (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      name TEXT NOT NULL,
      description TEXT,
      workspace_id INTEGER NOT NULL,
      type TEXT NOT NULL DEFAULT 'category'
    );
    CREATE TABLE IF NOT EXISTS brands (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      name TEXT NOT NULL,
      description TEXT,
      workspace_id INTEGER NOT NULL,
      type TEXT NOT NULL DEFAULT 'brand'
    );
    CREATE TABLE IF NOT EXISTS statuses (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      name TEXT NOT NULL,
      description TEXT,
      workspace_id INTEGER NOT NULL,
      type TEXT NOT NULL DEFAULT 'status',
      slug TEXT NOT NULL DEFAULT 'in-stock',
      color TEXT NOT NULL DEFAULT '#5E629B',
      bg TEXT NOT NULL DEFAULT '#EDEDF7'
    );
    CREATE TABLE IF NOT EXISTS items (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      internal_id TEXT NOT NULL,
      title TEXT NOT NULL,
      category_id INTEGER,
      brand_id INTEGER,
      status_id INTEGER,
      responsible_user_id INTEGER,
      building_site_id INTEGER,
      storage_id INTEGER,
      workspace_id INTEGER NOT NULL,
      serial_number TEXT,
      cost REAL,
      quantitative INTEGER NOT NULL DEFAULT 0,
      quantity REAL,
      unit TEXT,
      comment TEXT,
      qr_code TEXT,
      notify_date INTEGER,
      created_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS item_photos (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      item_id INTEGER NOT NULL,
      url TEXT NOT NULL,
      is_title INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS item_documents (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      item_id INTEGER NOT NULL,
      name TEXT NOT NULL,
      url TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS item_comments (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      item_id INTEGER NOT NULL,
      user_id INTEGER NOT NULL,
      text TEXT NOT NULL,
      active INTEGER NOT NULL DEFAULT 1,
      created_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS transfers (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      code TEXT,
      item_id INTEGER NOT NULL,
      from_user_id INTEGER NOT NULL,
      to_user_id INTEGER NOT NULL,
      to_storage_id INTEGER,
      building_site_id INTEGER,
      workspace_id INTEGER NOT NULL,
      quantity REAL,
      status TEXT NOT NULL DEFAULT 'pending',
      photo_url TEXT,
      comment TEXT,
      no_confirmation INTEGER NOT NULL DEFAULT 0,
      created_at INTEGER NOT NULL,
      completed_at INTEGER
    );
    CREATE TABLE IF NOT EXISTS history_entries (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      workspace_id INTEGER NOT NULL,
      item_id INTEGER NOT NULL,
      type TEXT NOT NULL,
      actor_user_id INTEGER NOT NULL,
      from_label TEXT,
      to_label TEXT,
      quantity_delta REAL,
      comment TEXT,
      prev_hash TEXT,
      hash TEXT NOT NULL,
      created_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS inventory_sessions (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      number TEXT NOT NULL,
      workspace_id INTEGER NOT NULL,
      status TEXT NOT NULL DEFAULT 'in_progress',
      started_by INTEGER NOT NULL,
      created_at INTEGER NOT NULL,
      completed_at INTEGER
    );
    CREATE TABLE IF NOT EXISTS inventory_results (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      session_id INTEGER NOT NULL,
      item_id INTEGER NOT NULL,
      expected_qty REAL,
      actual_qty REAL,
      checked INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS notifications (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      user_id INTEGER NOT NULL,
      item_id INTEGER,
      type TEXT NOT NULL,
      title TEXT,
      text TEXT NOT NULL,
      read INTEGER NOT NULL DEFAULT 0,
      created_at INTEGER NOT NULL
    );
  `);
}

export function getSqlite() {
  if (!sqlite) {
    fs.mkdirSync(path.dirname(env.databasePath), { recursive: true });
    sqlite = new Database(env.databasePath);
    sqlite.pragma("journal_mode = WAL");
    sqlite.pragma("foreign_keys = ON");
    ensureSchema(sqlite);
    migrateExisting(sqlite);
  }
  return sqlite;
}

export function getDb() {
  if (!instance) {
    instance = drizzle(getSqlite(), { schema: fullSchema });
  }
  return instance;
}
