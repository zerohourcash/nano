import { getDb } from "./connection";
import { transfers, items } from "@db/schema";
import { and, desc, eq, inArray } from "drizzle-orm";

export async function findOutgoingTransfers(fromUserId: number) {
  return getDb().query.transfers.findMany({
    where: and(eq(transfers.fromUserId, fromUserId), inArray(transfers.status, ["draft", "pending"])),
    with: {
      item: { with: { photos: true, status: true } },
      fromUser: true,
      toUser: true,
      toStorage: true,
    },
    orderBy: (t, { desc: d }) => [d(t.createdAt)],
  });
}

export async function findIncomingTransfers(toUserId: number) {
  return getDb().query.transfers.findMany({
    where: and(eq(transfers.toUserId, toUserId), eq(transfers.status, "pending")),
    with: {
      item: { with: { photos: true, status: true } },
      fromUser: true,
      toUser: true,
      toStorage: true,
    },
    orderBy: (t, { desc: d }) => [d(t.createdAt)],
  });
}

export async function findTransferById(id: number) {
  return getDb().query.transfers.findFirst({
    where: eq(transfers.id, id),
    with: { item: { with: { photos: true } }, fromUser: true, toUser: true, toStorage: true },
  });
}

export async function nextTransferCode(workspaceId: number) {
  const db = getDb();
  const rows = await db
    .select({ code: transfers.code })
    .from(transfers)
    .where(eq(transfers.workspaceId, workspaceId))
    .orderBy(desc(transfers.id))
    .limit(50);
  let max = 0;
  for (const r of rows) {
    const n = parseInt((r.code ?? "").replace("ПП-", ""), 10);
    if (!Number.isNaN(n) && n > max) max = n;
  }
  return `ПП-${String(max + 1).padStart(4, "0")}`;
}

export async function createTransfer(data: {
  workspaceId: number;
  itemId: number;
  fromUserId: number;
  toUserId: number;
  toStorageId?: number | null;
  buildingSiteId?: number | null;
  quantity?: number | null;
  comment?: string | null;
  noConfirmation?: boolean;
  status?: "draft" | "pending";
}) {
  const db = getDb();
  const code = await nextTransferCode(data.workspaceId);
  const [row] = await db
    .insert(transfers)
    .values({
      code,
      workspaceId: data.workspaceId,
      itemId: data.itemId,
      fromUserId: data.fromUserId,
      toUserId: data.toUserId,
      toStorageId: data.toStorageId ?? null,
      buildingSiteId: data.buildingSiteId ?? null,
      quantity: data.quantity ?? null,
      comment: data.comment ?? null,
      noConfirmation: data.noConfirmation ?? false,
      status: data.status ?? "pending",
    })
    .returning({ id: transfers.id });
  return findTransferById(row!.id);
}

export async function setTransferStatus(
  id: number,
  status: "pending" | "accepted" | "rejected",
  extra?: { photoUrl?: string | null; comment?: string | null },
) {
  const db = getDb();
  await db
    .update(transfers)
    .set({
      status,
      completedAt: status === "accepted" || status === "rejected" ? new Date() : null,
      ...(extra?.photoUrl !== undefined ? { photoUrl: extra.photoUrl } : {}),
      ...(extra?.comment !== undefined ? { comment: extra.comment } : {}),
    })
    .where(eq(transfers.id, id));
  return findTransferById(id);
}

export async function transferCounts(userId: number) {
  const db = getDb();
  const outgoing = await db
    .select({ id: transfers.id })
    .from(transfers)
    .where(and(eq(transfers.fromUserId, userId), inArray(transfers.status, ["draft", "pending"])));
  const incoming = await db
    .select({ id: transfers.id })
    .from(transfers)
    .where(and(eq(transfers.toUserId, userId), eq(transfers.status, "pending")));
  return { outgoing: outgoing.length, incoming: incoming.length };
}

/** Смена ответственного/склада инструмента при принятии передачи. */
export async function applyTransferToItem(
  itemId: number,
  data: {
    responsibleUserId: number | null;
    storageId?: number | null;
    buildingSiteId?: number | null;
    statusId?: number | null;
  },
) {
  await getDb().update(items).set(data).where(eq(items.id, itemId));
}
