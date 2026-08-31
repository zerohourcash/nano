import { getDb } from "./connection";
import { inventorySessions, inventoryResults, items } from "@db/schema";
import { and, eq } from "drizzle-orm";

export async function findInventorySessions(workspaceId: number) {
  const db = getDb();
  const sessions = await db.query.inventorySessions.findMany({
    where: eq(inventorySessions.workspaceId, workspaceId),
    with: { starter: true, results: true },
    orderBy: (s, { desc }) => [desc(s.createdAt)],
  });
  return sessions.map((s) => ({
    ...s,
    totalItems: s.results.length,
    checkedItems: s.results.filter((r) => r.checked).length,
  }));
}

export async function findInventorySessionById(id: number) {
  return getDb().query.inventorySessions.findFirst({
    where: eq(inventorySessions.id, id),
    with: {
      starter: true,
      results: {
        with: { item: { with: { photos: true, category: true, status: true } } },
      },
    },
  });
}

export async function createInventorySession(data: {
  workspaceId: number;
  startedBy: number;
  storageId?: number;
}) {
  const db = getDb();
  const countRows = await db
    .select({ id: inventorySessions.id })
    .from(inventorySessions)
    .where(eq(inventorySessions.workspaceId, data.workspaceId));
  const number = `ИНВ-${String(countRows.length + 1).padStart(3, "0")}`;

  const [row] = await db
    .insert(inventorySessions)
    .values({ number, workspaceId: data.workspaceId, startedBy: data.startedBy })
    .returning({ id: inventorySessions.id });
  const id = row!.id;

  // Заполняем ожидаемые позиции текущим остатком (по складу, если указан).
  const conds = [eq(items.workspaceId, data.workspaceId)];
  if (data.storageId) conds.push(eq(items.storageId, data.storageId));
  const stock = await db
    .select({ id: items.id, quantity: items.quantity, quantitative: items.quantitative })
    .from(items)
    .where(and(...conds));
  if (stock.length) {
    await db.insert(inventoryResults).values(
      stock.map((it) => ({
        sessionId: id,
        itemId: it.id,
        expectedQty: it.quantitative ? (it.quantity ?? 0) : 1,
        checked: false,
      })),
    );
  }
  return findInventorySessionById(id);
}

export async function checkInventoryItem(
  sessionId: number,
  itemId: number,
  actualQty?: number | null,
) {
  const db = getDb();
  await db
    .update(inventoryResults)
    .set({ checked: true, ...(actualQty !== undefined ? { actualQty } : {}) })
    .where(and(eq(inventoryResults.sessionId, sessionId), eq(inventoryResults.itemId, itemId)));
  return findInventorySessionById(sessionId);
}

export async function uncheckInventoryItem(sessionId: number, itemId: number) {
  const db = getDb();
  await db
    .update(inventoryResults)
    .set({ checked: false })
    .where(and(eq(inventoryResults.sessionId, sessionId), eq(inventoryResults.itemId, itemId)));
  return findInventorySessionById(sessionId);
}

export async function completeInventorySession(sessionId: number) {
  const db = getDb();
  await db
    .update(inventorySessions)
    .set({ status: "completed", completedAt: new Date() })
    .where(eq(inventorySessions.id, sessionId));
  return findInventorySessionById(sessionId);
}
