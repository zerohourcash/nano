import "dotenv/config";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const appRoot = path.resolve(here, "../..");

function optional(name: string, fallback: string): string {
  return process.env[name] || fallback;
}

export const env = {
  appId: optional("APP_ID", "meshkeeper-local"),
  appSecret: optional("APP_SECRET", "meshkeeper-dev-secret"),
  isProduction: process.env.NODE_ENV === "production",
  databasePath: path.resolve(appRoot, optional("DATABASE_PATH", "data/meshkeeper.db")),
};
