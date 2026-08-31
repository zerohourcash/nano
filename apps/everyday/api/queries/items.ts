import { getDb } from "./connection";
import { items, itemPhotos, itemDocuments, itemComments } from "@db/schema";
import type { Item } from "@db/schema";
import { and, asc, desc, eq, isNotNull, isNull, like, or, SQL } from "drizzle-orm";

export type ItemListFilter = {
  workspaceId: number;
  search?: string;
  userId?: number;
  buildingSiteId?: number;
  storageId?: number;
  categoryId?: number;
  brandId?: number;
  statusId?: number;
  hasQr?: boolean;
  onlyMineUserId?: number;
  page?: number;
  limit?: number;
  sort?: "createdAt_desc" | "createdAt_asc" | "title_asc" | "title_desc" | "internalId_asc";
};

export async function findItems(filter: ItemListFilter) {
  const db = getDb();
  const conds: SQL[] = [eq(items.workspaceId, filter.workspaceId)];
  if (filter.search) {
    const q = `%${filter.search}%`;
    conds.push(
      or(like(items.title, q), like(items.internalId, q), like(items.serialNumber, q))!,
    );
  }
  if (filter.userId) conds.push(eq(items.responsibleUserId, filter.userId));
  if (filter.onlyMineUserId) conds.push(eq(items.responsibleUserId, filter.onlyMineUserId));
  if (filter.buildingSiteId) conds.push(eq(items.buildingSiteId, filter.buildingSiteId));
  if (filter.storageId) conds.push(eq(items.storageId, filter.storageId));
  if (filter.categoryId) conds.push(eq(items.categoryId, filter.categoryId));
  if (filter.brandId) conds.push(eq(items.brandId, filter.brandId));
  if (filter.statusId) conds.push(eq(items.statusId, filter.statusId));
  if (filter.hasQr === true) conds.push(isNotNull(items.qrCode));
  if (filter.hasQr === false) conds.push(isNull(items.qrCode));

  const orderBy =
    filter.sort === "createdAt_asc"
      ? [asc(items.createdAt)]
      : filter.sort === "title_asc"
        ? [asc(items.title)]
        : filter.sort === "title_desc"
          ? [desc(items.title)]
          : filter.sort === "internalId_asc"
            ? [asc(items.internalId)]
            : [desc(items.createdAt), desc(items.id)];

  const page = Math.max(1, filter.page ?? 1);
  const limit = Math.min(500, Math.max(1, filter.limit ?? 20));

  const rows = await db.query.items.findMany({
    where: and(...conds),
    with: {
      category: true,
      brand: true,
      status: true,
      responsible: true,
      buildingSite: true,
      storage: true,
      photos: true,
    },
    orderBy,
    limit: limit + 1,
    offset: (page - 1) * limit,
  });

  const hasMore = rows.length > limit;
  return { rows: rows.slice(0, limit), page, limit, hasMore };
}

export async function countItems(workspaceId: number) {
  const rows = await getDb()
    .select({ id: items.id })
    .from(items)
    .where(eq(items.workspaceId, workspaceId));
  return rows.length;
}

export async function findItemById(id: number) {
  return getDb().query.items.findFirst({
    where: eq(items.id, id),
    with: {
      category: true,
      brand: true,
      status: true,
      responsible: true,
      buildingSite: true,
      storage: true,
      photos: true,
      documents: true,
      comments: { with: { user: true }, orderBy: (c, { desc: d }) => [d(c.createdAt)] },
    },
  });
}

/** Поиск по QR-коду или внутреннему номеру (ВН-0142). */
export async function findItemByCode(code: string) {
  const raw = code.trim();
  if (!raw) return null;
  const normalized = raw.replace(/\s+/g, "").toUpperCase();
  const db = getDb();
  const byQr = await db.query.items.findFirst({
    where: or(eq(items.qrCode, raw), eq(items.qrCode, normalized), eq(items.internalId, raw), eq(items.internalId, normalized)),
    with: {
      category: true,
      brand: true,
      status: true,
      responsible: true,
      buildingSite: true,
      storage: true,
      photos: true,
    },
  });
  if (byQr) return byQr;
  return db.query.items.findFirst({
    where: or(like(items.internalId, `%${normalized}%`), like(items.qrCode, `%${normalized}%`)),
    with: {
      category: true,
      brand: true,
      status: true,
      responsible: true,
      buildingSite: true,
      storage: true,
      photos: true,
    },
  });
}

export async function nextInternalId(workspaceId: number, prefix: string) {
  const db = getDb();
  const rows = await db
    .select({ internalId: items.internalId })
    .from(items)
    .where(and(eq(items.workspaceId, workspaceId), like(items.internalId, `${prefix}%`)))
    .orderBy(desc(items.id))
    .limit(50);
  let max = 0;
  for (const r of rows) {
    const n = parseInt(r.internalId.replace(prefix, ""), 10);
    if (!Number.isNaN(n) && n > max) max = n;
  }
  return `${prefix}${String(max + 1).padStart(4, "0")}`;
}

/** Снимок приходит строкой (старые клиенты) либо парой «оригинал + миниатюра». */
export type PhotoInput = string | { url: string; thumbUrl: string };

export async function createItem(
  data: Omit<typeof items.$inferInsert, "id"> & { photos?: PhotoInput[] },
) {
  const db = getDb();
  const { photos: photoUrls, ...itemData } = data;
  const [row] = await db.insert(items).values(itemData).returning({ id: items.id });
  const id = row!.id;
  if (photoUrls?.length) {
    await db.insert(itemPhotos).values(
      photoUrls.map((photo, i) => {
        const url = typeof photo === "string" ? photo : photo.url;
        const thumbUrl = typeof photo === "string" ? photo : photo.thumbUrl;
        return { itemId: id, url, thumbUrl, isTitle: i === 0 };
      }),
    );
  }
  return findItemById(id);
}

export async function updateItem(
  id: number,
  data: Partial<Omit<Item, "id" | "createdAt">>,
) {
  await getDb().update(items).set(data).where(eq(items.id, id));
  return findItemById(id);
}

export async function deleteItem(id: number) {
  const db = getDb();
  await db.delete(itemPhotos).where(eq(itemPhotos.itemId, id));
  await db.delete(itemDocuments).where(eq(itemDocuments.itemId, id));
  await db.delete(itemComments).where(eq(itemComments.itemId, id));
  await db.delete(items).where(eq(items.id, id));
  return { ok: true };
}

export async function addItemPhoto(itemId: number, url: string, isTitle = false) {
  const db = getDb();
  const [row] = await db.insert(itemPhotos).values({ itemId, url, isTitle }).returning({ id: itemPhotos.id });
  return db.query.itemPhotos.findFirst({ where: eq(itemPhotos.id, row!.id) });
}

export async function addItemComment(itemId: number, userId: number, text: string) {
  const db = getDb();
  const [row] = await db.insert(itemComments).values({ itemId, userId, text }).returning({ id: itemComments.id });
  return db.query.itemComments.findFirst({
    where: eq(itemComments.id, row!.id),
    with: { user: true },
  });
}
