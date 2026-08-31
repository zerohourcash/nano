import { createHash, randomBytes, timingSafeEqual } from "node:crypto";

export function hashPassword(password: string): string {
  const salt = randomBytes(16).toString("hex");
  const digest = createHash("sha256").update(`${salt}:${password}`).digest("hex");
  return `${salt}$${digest}`;
}

export function verifyPassword(password: string, stored: string): boolean {
  const sep = stored.indexOf("$");
  if (sep <= 0) return false;
  const salt = stored.slice(0, sep);
  const digest = stored.slice(sep + 1);
  const check = createHash("sha256").update(`${salt}:${password}`).digest("hex");
  if (digest.length !== check.length) return false;
  return timingSafeEqual(Buffer.from(digest, "hex"), Buffer.from(check, "hex"));
}

export function publicUser<T extends { passwordHash?: string | null }>(user: T) {
  const { passwordHash: _ignored, ...rest } = user;
  void _ignored;
  return rest;
}
