import { getDb } from "./connection";
import { workspaces, storages, buildingSites, categories, brands, statuses } from "@db/schema";
import { and, eq } from "drizzle-orm";

// ─── Рабочие пространства ────────────────────────────────────────────────────

export async function findWorkspaces() {
  return getDb().query.workspaces.findMany({ orderBy: (w, { asc }) => [asc(w.id)] });
}

export async function getDefaultWorkspaceId(): Promise<number> {
  const first = await getDb().query.workspaces.findFirst({ orderBy: (w, { asc }) => [asc(w.id)] });
  if (!first) throw new Error("No workspace found — run `npx tsx db/seed.ts` first");
  return first.id;
}

export async function findWorkspaceById(id: number) {
  return getDb().query.workspaces.findFirst({ where: eq(workspaces.id, id) });
}

export async function createWorkspace(data: {
  name: string;
  timezone?: string;
  internalIdPrefix?: string;
  comment?: string;
}) {
  const db = getDb();
  const [row] = await db.insert(workspaces).values(data).returning({ id: workspaces.id });
  return findWorkspaceById(row!.id);
}

export async function updateWorkspace(
  id: number,
  data: Partial<{ name: string; timezone: string; internalIdPrefix: string; comment: string | null }>,
) {
  await getDb().update(workspaces).set(data).where(eq(workspaces.id, id));
  return findWorkspaceById(id);
}

export async function deleteWorkspace(id: number) {
  await getDb().delete(workspaces).where(eq(workspaces.id, id));
  return { ok: true };
}

// ─── Склады ──────────────────────────────────────────────────────────────────

export async function findStorages(workspaceId: number) {
  return getDb().query.storages.findMany({
    where: eq(storages.workspaceId, workspaceId),
    with: { responsible: true },
    orderBy: (s, { asc }) => [asc(s.id)],
  });
}

export async function createStorage(data: {
  name: string;
  workspaceId: number;
  responsibleUserId?: number | null;
  address?: string;
}) {
  const db = getDb();
  const [row] = await db.insert(storages).values(data).returning({ id: storages.id });
  return db.query.storages.findFirst({ where: eq(storages.id, row!.id), with: { responsible: true } });
}

export async function updateStorage(
  id: number,
  data: Partial<{ name: string; responsibleUserId: number | null; address: string | null }>,
) {
  const db = getDb();
  await db.update(storages).set(data).where(eq(storages.id, id));
  return db.query.storages.findFirst({ where: eq(storages.id, id), with: { responsible: true } });
}

export async function deleteStorage(id: number) {
  await getDb().delete(storages).where(eq(storages.id, id));
  return { ok: true };
}

// ─── Объекты строительства ───────────────────────────────────────────────────

export async function findBuildingSites(workspaceId: number) {
  return getDb().query.buildingSites.findMany({
    where: eq(buildingSites.workspaceId, workspaceId),
    with: { responsible: true },
    orderBy: (s, { asc }) => [asc(s.id)],
  });
}

export async function createBuildingSite(data: {
  name: string;
  workspaceId: number;
  responsibleUserId?: number | null;
}) {
  const db = getDb();
  const [row] = await db.insert(buildingSites).values(data).returning({ id: buildingSites.id });
  return db.query.buildingSites.findFirst({
    where: eq(buildingSites.id, row!.id),
    with: { responsible: true },
  });
}

export async function updateBuildingSite(
  id: number,
  data: Partial<{ name: string; responsibleUserId: number | null }>,
) {
  const db = getDb();
  await db.update(buildingSites).set(data).where(eq(buildingSites.id, id));
  return db.query.buildingSites.findFirst({
    where: eq(buildingSites.id, id),
    with: { responsible: true },
  });
}

export async function deleteBuildingSite(id: number) {
  await getDb().delete(buildingSites).where(eq(buildingSites.id, id));
  return { ok: true };
}

// ─── Справочники ─────────────────────────────────────────────────────────────

export type DictKind = "categories" | "brands" | "statuses";

export async function findCategories(workspaceId: number) {
  return getDb().query.categories.findMany({
    where: eq(categories.workspaceId, workspaceId),
    orderBy: (c, { asc }) => [asc(c.id)],
  });
}

export async function findBrands(workspaceId: number) {
  return getDb().query.brands.findMany({
    where: eq(brands.workspaceId, workspaceId),
    orderBy: (b, { asc }) => [asc(b.id)],
  });
}

export async function findStatuses(workspaceId: number) {
  return getDb().query.statuses.findMany({
    where: eq(statuses.workspaceId, workspaceId),
    orderBy: (s, { asc }) => [asc(s.id)],
  });
}

export async function createCategory(data: { name: string; description?: string; workspaceId: number }) {
  const db = getDb();
  const [row] = await db.insert(categories).values(data).returning({ id: categories.id });
  return db.query.categories.findFirst({ where: eq(categories.id, row!.id) });
}

export async function createBrand(data: { name: string; description?: string; workspaceId: number }) {
  const db = getDb();
  const [row] = await db.insert(brands).values(data).returning({ id: brands.id });
  return db.query.brands.findFirst({ where: eq(brands.id, row!.id) });
}

export async function createStatus(data: {
  name: string;
  description?: string;
  workspaceId: number;
  slug?: string;
  color?: string;
  bg?: string;
}) {
  const db = getDb();
  const [row] = await db.insert(statuses).values(data).returning({ id: statuses.id });
  return db.query.statuses.findFirst({ where: eq(statuses.id, row!.id) });
}

export async function findStatusBySlug(workspaceId: number, slug: string) {
  return getDb().query.statuses.findFirst({
    where: and(eq(statuses.workspaceId, workspaceId), eq(statuses.slug, slug)),
  });
}

export async function updateCategory(id: number, data: { name?: string; description?: string | null }) {
  const db = getDb();
  await db.update(categories).set(data).where(eq(categories.id, id));
  return db.query.categories.findFirst({ where: eq(categories.id, id) });
}

export async function updateBrand(id: number, data: { name?: string; description?: string | null }) {
  const db = getDb();
  await db.update(brands).set(data).where(eq(brands.id, id));
  return db.query.brands.findFirst({ where: eq(brands.id, id) });
}

export async function updateStatus(
  id: number,
  data: { name?: string; description?: string | null; slug?: string; color?: string; bg?: string },
) {
  const db = getDb();
  await db.update(statuses).set(data).where(eq(statuses.id, id));
  return db.query.statuses.findFirst({ where: eq(statuses.id, id) });
}

export async function deleteCategory(id: number) {
  await getDb().delete(categories).where(eq(categories.id, id));
  return { ok: true };
}

export async function deleteBrand(id: number) {
  await getDb().delete(brands).where(eq(brands.id, id));
  return { ok: true };
}

export async function deleteStatus(id: number) {
  await getDb().delete(statuses).where(eq(statuses.id, id));
  return { ok: true };
}
