import { getDb } from "./connection";
import { users, userWorkspaces } from "@db/schema";
import { DEFAULT_ROLE_RIGHTS } from "@db/schema";
import type { RoleRights } from "@db/schema";
import { eq } from "drizzle-orm";

/** Телефон демо-пользователя (Алексей Кузнецов, кладовщик). */
export const DEMO_USER_PHONE = "+7 921 555-01-42";

export async function getDemoUser() {
  const db = getDb();
  const byPhone = await db.query.users.findFirst({
    where: eq(users.phone, DEMO_USER_PHONE),
  });
  if (byPhone) return byPhone;
  return db.query.users.findFirst();
}

export async function findUserByPhone(phone: string) {
  const digits = phone.replace(/\D/g, "");
  const all = await getDb().query.users.findMany();
  return all.find((u) => u.phone.replace(/\D/g, "") === digits) ?? null;
}

export async function findUsers(workspaceId?: number) {
  const db = getDb();
  if (!workspaceId) return db.query.users.findMany({ orderBy: (u, { asc }) => [asc(u.id)] });
  const links = await db.query.userWorkspaces.findMany({
    where: eq(userWorkspaces.workspaceId, workspaceId),
    with: { user: true },
  });
  return links.map((l) => l.user);
}

export async function findUserById(id: number) {
  return getDb().query.users.findFirst({ where: eq(users.id, id) });
}

export async function createUser(data: {
  fullName: string;
  position?: string;
  phone: string;
  avatarUrl?: string;
  passwordHash?: string | null;
  roleRights?: RoleRights;
  workspaceIds?: number[];
}) {
  const db = getDb();
  const [inserted] = await db
    .insert(users)
    .values({
      fullName: data.fullName,
      position: data.position,
      phone: data.phone,
      avatarUrl: data.avatarUrl,
      status: "active",
      passwordHash: data.passwordHash ?? null,
      roleRights: data.roleRights ?? DEFAULT_ROLE_RIGHTS,
    })
    .returning({ id: users.id });
  const id = inserted!.id;
  if (data.workspaceIds?.length) {
    await db
      .insert(userWorkspaces)
      .values(data.workspaceIds.map((workspaceId) => ({ userId: id, workspaceId })));
  }
  return findUserById(id);
}

export async function updateUser(
  id: number,
  data: Partial<{
    fullName: string;
    position: string | null;
    phone: string;
    avatarUrl: string | null;
    status: "active" | "invited" | "disabled";
    passwordHash: string | null;
    roleRights: RoleRights;
  }>,
) {
  await getDb().update(users).set(data).where(eq(users.id, id));
  return findUserById(id);
}

export async function deleteUser(id: number) {
  const db = getDb();
  await db.delete(userWorkspaces).where(eq(userWorkspaces.userId, id));
  await db.delete(users).where(eq(users.id, id));
  return { ok: true };
}

export async function inviteUser(data: {
  fullName: string;
  phone: string;
  position?: string;
  workspaceId: number;
}) {
  const db = getDb();
  const [row] = await db
    .insert(users)
    .values({
      fullName: data.fullName,
      phone: data.phone,
      position: data.position,
      status: "active",
      roleRights: DEFAULT_ROLE_RIGHTS,
    })
    .returning({ id: users.id });
  const id = row!.id;
  await db.insert(userWorkspaces).values({ userId: id, workspaceId: data.workspaceId });
  return findUserById(id);
}

export async function workspacesOfUser(userId: number) {
  const links = await getDb().query.userWorkspaces.findMany({
    where: eq(userWorkspaces.userId, userId),
    with: { workspace: true },
  });
  return links.map((l) => l.workspace);
}
