import fs from 'node:fs/promises'

export default async function globalTeardown() {
  const db = process.env.MESHKEEPER_E2E_DB
  if (!db) return
  await Promise.all(['', '-wal', '-shm'].map((suffix) => fs.rm(db + suffix, { force: true })))
}
