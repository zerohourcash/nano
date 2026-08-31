import { randomUUID } from "node:crypto";
import { getDb } from "./connection";
import { historyEntries } from "@db/schema";
import type { HistoryEntry } from "@db/schema";
import { and, eq, gte, lte, inArray } from "drizzle-orm";

export type HistoryType = HistoryEntry["type"];

type DbOrTx = Pick<ReturnType<typeof getDb>, "select" | "insert">;

/** Append-only запись в журнал операций (аудит-лог без криптографии). */
export async function appendHistory(
  db: DbOrTx,
  entry: {
    workspaceId: number;
    itemId: number;
    type: HistoryType;
    actorUserId: number;
    fromLabel?: string | null;
    toLabel?: string | null;
    quantityDelta?: number | null;
    comment?: string | null;
    createdAt?: Date;
  },
): Promise<void> {
  const createdAt = entry.createdAt ?? new Date();
  await db.insert(historyEntries).values({
    workspaceId: entry.workspaceId,
    itemId: entry.itemId,
    type: entry.type,
    actorUserId: entry.actorUserId,
    fromLabel: entry.fromLabel ?? null,
    toLabel: entry.toLabel ?? null,
    quantityDelta: entry.quantityDelta ?? null,
    comment: entry.comment ?? null,
    opId: randomUUID().replace(/-/g, ""),
    createdAt,
  });
}

export type HistoryFilter = {
  workspaceId: number;
  types?: HistoryType[];
  dateFrom?: Date;
  dateTo?: Date;
  itemId?: number;
  limit?: number;
};

export async function findHistory(filter: HistoryFilter) {
  const db = getDb();
  const conds = [eq(historyEntries.workspaceId, filter.workspaceId)];
  if (filter.types?.length) conds.push(inArray(historyEntries.type, filter.types));
  if (filter.dateFrom) conds.push(gte(historyEntries.createdAt, filter.dateFrom));
  if (filter.dateTo) conds.push(lte(historyEntries.createdAt, filter.dateTo));
  if (filter.itemId) conds.push(eq(historyEntries.itemId, filter.itemId));
  return db.query.historyEntries.findMany({
    where: and(...conds),
    with: {
      item: { with: { photos: true } },
      actor: true,
    },
    orderBy: (h, { desc: d }) => [d(h.createdAt), d(h.id)],
    limit: filter.limit ?? 200,
  });
}

export async function findItemHistory(itemId: number) {
  return getDb().query.historyEntries.findMany({
    where: eq(historyEntries.itemId, itemId),
    with: { actor: true },
    orderBy: (h, { desc: d }) => [d(h.createdAt), d(h.id)],
  });
}
