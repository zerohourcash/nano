import { getDb } from "./connection";
import { notifications, historyEntries, items } from "@db/schema";
import { and, desc, eq, gte, inArray, lte } from "drizzle-orm";

// ─── Уведомления ─────────────────────────────────────────────────────────────

export async function findNotifications(userId: number) {
  return getDb().query.notifications.findMany({
    where: eq(notifications.userId, userId),
    with: { item: { with: { photos: true } } },
    orderBy: (n, { desc: d }) => [d(n.createdAt)],
    limit: 100,
  });
}

export async function unreadCount(userId: number) {
  const rows = await getDb()
    .select({ id: notifications.id })
    .from(notifications)
    .where(and(eq(notifications.userId, userId), eq(notifications.read, false)));
  return rows.length;
}

export async function markNotificationRead(id: number) {
  const db = getDb();
  await db.update(notifications).set({ read: true }).where(eq(notifications.id, id));
  return db.query.notifications.findFirst({ where: eq(notifications.id, id) });
}

export async function markAllNotificationsRead(userId: number) {
  await getDb()
    .update(notifications)
    .set({ read: true })
    .where(and(eq(notifications.userId, userId), eq(notifications.read, false)));
  return { ok: true };
}

export async function createNotification(data: {
  userId: number;
  itemId?: number | null;
  type: string;
  title?: string;
  text: string;
}) {
  const db = getDb();
  const [row] = await db
    .insert(notifications)
    .values({
      userId: data.userId,
      itemId: data.itemId ?? null,
      type: data.type,
      title: data.title ?? null,
      text: data.text,
    })
    .returning({ id: notifications.id });
  return db.query.notifications.findFirst({ where: eq(notifications.id, row!.id) });
}

// ─── Отчёты ──────────────────────────────────────────────────────────────────

/** Отчёт «по ответственным»: пользователь → инструменты. */
export async function reportByUsers(workspaceId: number) {
  const db = getDb();
  const rows = await db.query.items.findMany({
    where: eq(items.workspaceId, workspaceId),
    with: { responsible: true, status: true, category: true },
  });
  const byUser = new Map<number | null, typeof rows>();
  for (const row of rows) {
    const key = row.responsibleUserId ?? null;
    const arr = byUser.get(key) ?? [];
    arr.push(row);
    byUser.set(key, arr);
  }
  return [...byUser.entries()].map(([userId, list]) => ({
    userId,
    user: list[0]?.responsible ?? null,
    itemsCount: list.length,
    totalCost: list.reduce((sum, it) => sum + (it.cost ?? 0), 0),
    items: list,
  }));
}

/** Отчёт «поступление/списание» за диапазон дат. */
export async function reportQuantityTransactions(
  workspaceId: number,
  dateFrom?: Date,
  dateTo?: Date,
) {
  const conds = [
    eq(historyEntries.workspaceId, workspaceId),
    inArray(historyEntries.type, ["write_off", "replenish"]),
  ];
  if (dateFrom) conds.push(gte(historyEntries.createdAt, dateFrom));
  if (dateTo) conds.push(lte(historyEntries.createdAt, dateTo));
  return getDb().query.historyEntries.findMany({
    where: and(...conds),
    with: { item: true, actor: true },
    orderBy: (h, { desc: d }) => [d(h.createdAt)],
  });
}

/** Отчёт «все инструменты». */
export async function reportAllItems(workspaceId: number) {
  return getDb().query.items.findMany({
    where: eq(items.workspaceId, workspaceId),
    with: {
      category: true,
      brand: true,
      status: true,
      responsible: true,
      buildingSite: true,
      storage: true,
      photos: true,
    },
    orderBy: [desc(items.createdAt)],
  });
}
