import { z } from "zod";
import { createRouter, publicQuery } from "./middleware";
import { findUsers, createUser, updateUser, deleteUser, inviteUser } from "./queries/users";
import {
  findWorkspaces,
  findWorkspaceById,
  createWorkspace,
  updateWorkspace,
  deleteWorkspace,
  findStorages,
  createStorage,
  updateStorage,
  deleteStorage,
  findBuildingSites,
  createBuildingSite,
  updateBuildingSite,
  deleteBuildingSite,
  findCategories,
  findBrands,
  findStatuses,
  createCategory,
  createBrand,
  createStatus,
  updateCategory,
  updateBrand,
  updateStatus,
  deleteCategory,
  deleteBrand,
  deleteStatus,
  getDefaultWorkspaceId,
} from "./queries/catalog";
import { DEFAULT_ROLE_RIGHTS } from "@db/schema";

const roleRightsSchema = z.object({
  viewItems: z.boolean(),
  createItems: z.boolean(),
  editItems: z.boolean(),
  deleteItems: z.boolean(),
  transferItems: z.boolean(),
  acceptTransfers: z.boolean(),
  writeOff: z.boolean(),
  replenish: z.boolean(),
  inventory: z.boolean(),
  viewHistory: z.boolean(),
  viewReports: z.boolean(),
  manageUsers: z.boolean(),
  manageWorkspaces: z.boolean(),
  manageStorages: z.boolean(),
  manageSites: z.boolean(),
  manageDictionaries: z.boolean(),
});

const idInput = z.object({ id: z.number().int().positive() });
const wsInput = z.object({ workspaceId: z.number().int().positive().optional() });

/** Нормализованная запись справочника (categories/brands/statuses). */
type DictEntry = {
  id: number;
  name: string;
  description: string | null;
  workspaceId: number;
  type: string;
  slug: string | null;
  color: string | null;
  bg: string | null;
};

function normDict(row: unknown): DictEntry | undefined {
  if (!row || typeof row !== "object") return undefined;
  const r = row as Record<string, unknown>;
  return {
    id: r.id as number,
    name: r.name as string,
    description: (r.description as string | null) ?? null,
    workspaceId: r.workspaceId as number,
    type: r.type as string,
    slug: (r.slug as string | null) ?? null,
    color: (r.color as string | null) ?? null,
    bg: (r.bg as string | null) ?? null,
  };
}

export const adminRouter = createRouter({
  // ─── Пользователи ──────────────────────────────────────────────────────────

  users: createRouter({
    list: publicQuery.input(wsInput.optional()).query(async ({ input }) => {
      const workspaceId = input?.workspaceId ?? (await getDefaultWorkspaceId());
      return findUsers(workspaceId);
    }),

    create: publicQuery
      .input(
        z.object({
          fullName: z.string().min(1),
          position: z.string().optional(),
          phone: z.string().min(5),
          avatarUrl: z.string().optional(),
          roleRights: roleRightsSchema.optional(),
          workspaceIds: z.array(z.number().int().positive()).optional(),
        }),
      )
      .mutation(({ input }) => createUser(input)),

    update: publicQuery
      .input(
        z.object({
          id: z.number().int().positive(),
          fullName: z.string().min(1).optional(),
          position: z.string().nullable().optional(),
          phone: z.string().min(5).optional(),
          avatarUrl: z.string().nullable().optional(),
          status: z.enum(["active", "invited", "disabled"]).optional(),
          roleRights: roleRightsSchema.optional(),
          checkoutPolicy: z
            .object({
              allowedCategoryIds: z.array(z.number()).nullable().optional(),
              maxHours: z.number().nullable().optional(),
              requireApproval: z.boolean().optional(),
              allowNoDueDate: z.boolean().optional(),
            })
            .optional(),
        }),
      )
      .mutation(({ input }) => {
        const { id, ...data } = input;
        return updateUser(id, data);
      }),

    remove: publicQuery
      .input(z.object({ id: z.number().int().positive(), workspaceId: z.number().int().positive().optional() }))
      .mutation(async ({ input }) => {
        await deleteUser(input.id);
        // Узел возвращает, была ли запись удалена или только заблокирована:
        // историю подписанных операций стирать нельзя.
        return { ok: true, deleted: false, disabled: false, message: "" as string };
      }),

    invite: publicQuery
      .input(
        z.object({
          fullName: z.string().min(1),
          phone: z.string().min(5),
          position: z.string().optional(),
          workspaceId: z.number().int().positive().optional(),
        }),
      )
      .mutation(async ({ input }) => {
        const workspaceId = input.workspaceId ?? (await getDefaultWorkspaceId());
        return inviteUser({ ...input, workspaceId });
      }),

    /** Шаблон прав по умолчанию (16 булевых флагов). */
    defaultRights: publicQuery.query(() => DEFAULT_ROLE_RIGHTS),
  }),

  // ─── Рабочие пространства ──────────────────────────────────────────────────

  workspaces: createRouter({
    list: publicQuery.query(() => findWorkspaces()),

    create: publicQuery
      .input(
        z.object({
          name: z.string().min(1),
          timezone: z.string().optional(),
          internalIdPrefix: z.string().max(16).optional(),
          comment: z.string().optional(),
          syncUrl: z.string().optional(),
        }),
      )
      .mutation(({ input }) => createWorkspace(input)),

    update: publicQuery
      .input(
        z.object({
          id: z.number().int().positive(),
          name: z.string().min(1).optional(),
          timezone: z.string().optional(),
          internalIdPrefix: z.string().max(16).optional(),
          comment: z.string().nullable().optional(),
          syncUrl: z.string().nullable().optional(),
          requireWriteoffPhoto: z.boolean().optional(),
        }),
      )
      .mutation(({ input }) => {
        const { id, ...data } = input;
        return updateWorkspace(id, data);
      }),

    remove: publicQuery.input(idInput).mutation(({ input }) => deleteWorkspace(input.id)),

    createInvite: publicQuery
      .input(
        z.object({
          workspaceId: z.number().int().positive().optional(),
          role: z.string().optional(),
          maxUses: z.number().int().positive().optional(),
          expiresInHours: z.number().int().positive().optional(),
        }),
      )
      .mutation(async ({ input }) => {
        const token = crypto.randomUUID().replace(/-/g, "");
        const workspaceId = input.workspaceId ?? 1;
        const role = input.role ?? "member";
        const expiresAt = new Date().toISOString();
        return {
          token,
          workspaceId,
          role,
          expiresAt,
          workspace: await findWorkspaceById(workspaceId),
          payload: {
            v: 1 as const,
            t: "join" as const,
            ws: workspaceId,
            token,
            role,
            exp: expiresAt,
            name: "",
          },
        };
      }),

    invites: publicQuery
      .input(z.object({ workspaceId: z.number().int().positive().optional() }).optional())
      .query(async () => [] as Array<{
        id: number
        token: string
        role: string
        maxUses: number
        usedCount: number
        revoked: boolean
        createdAt: Date
        expiresAt: string | null
        expired: boolean
        usable: boolean
      }>),
  }),

  // ─── Склады ────────────────────────────────────────────────────────────────

  storages: createRouter({
    list: publicQuery.input(wsInput.optional()).query(async ({ input }) => {
      const workspaceId = input?.workspaceId ?? (await getDefaultWorkspaceId());
      return findStorages(workspaceId);
    }),

    create: publicQuery
      .input(
        z.object({
          name: z.string().min(1),
          workspaceId: z.number().int().positive().optional(),
          responsibleUserId: z.number().int().positive().nullable().optional(),
          address: z.string().optional(),
        }),
      )
      .mutation(async ({ input }) => {
        const workspaceId = input.workspaceId ?? (await getDefaultWorkspaceId());
        return createStorage({ ...input, workspaceId });
      }),

    update: publicQuery
      .input(
        z.object({
          id: z.number().int().positive(),
          name: z.string().min(1).optional(),
          responsibleUserId: z.number().int().positive().nullable().optional(),
          address: z.string().nullable().optional(),
        }),
      )
      .mutation(({ input }) => {
        const { id, ...data } = input;
        return updateStorage(id, data);
      }),

    remove: publicQuery.input(idInput).mutation(({ input }) => deleteStorage(input.id)),
  }),

  // ─── Объекты ───────────────────────────────────────────────────────────────

  buildingSites: createRouter({
    list: publicQuery.input(wsInput.optional()).query(async ({ input }) => {
      const workspaceId = input?.workspaceId ?? (await getDefaultWorkspaceId());
      return findBuildingSites(workspaceId);
    }),

    create: publicQuery
      .input(
        z.object({
          name: z.string().min(1),
          workspaceId: z.number().int().positive().optional(),
          responsibleUserId: z.number().int().positive().nullable().optional(),
        }),
      )
      .mutation(async ({ input }) => {
        const workspaceId = input.workspaceId ?? (await getDefaultWorkspaceId());
        return createBuildingSite({ ...input, workspaceId });
      }),

    update: publicQuery
      .input(
        z.object({
          id: z.number().int().positive(),
          name: z.string().min(1).optional(),
          responsibleUserId: z.number().int().positive().nullable().optional(),
        }),
      )
      .mutation(({ input }) => {
        const { id, ...data } = input;
        return updateBuildingSite(id, data);
      }),

    remove: publicQuery.input(idInput).mutation(({ input }) => deleteBuildingSite(input.id)),
  }),

  // ─── Справочники (categories / brands / statuses) ──────────────────────────

  dictionaries: createRouter({
    list: publicQuery
      .input(
        z.object({
          kind: z.enum(["categories", "brands", "statuses"]),
          workspaceId: z.number().int().positive().optional(),
        }),
      )
      .query(async ({ input }): Promise<DictEntry[]> => {
        const workspaceId = input.workspaceId ?? (await getDefaultWorkspaceId());
        const rows =
          input.kind === "categories"
            ? await findCategories(workspaceId)
            : input.kind === "brands"
              ? await findBrands(workspaceId)
              : await findStatuses(workspaceId);
        return rows.map(normDict) as DictEntry[];
      }),

    create: publicQuery
      .input(
        z.object({
          kind: z.enum(["categories", "brands", "statuses"]),
          name: z.string().min(1),
          description: z.string().optional(),
          workspaceId: z.number().int().positive().optional(),
          slug: z.string().optional(),
          color: z.string().optional(),
          bg: z.string().optional(),
        }),
      )
      .mutation(async ({ input }): Promise<DictEntry | undefined> => {
        const { kind, ...data } = input;
        const workspaceId = data.workspaceId ?? (await getDefaultWorkspaceId());
        const row =
          kind === "categories"
            ? await createCategory({ ...data, workspaceId })
            : kind === "brands"
              ? await createBrand({ ...data, workspaceId })
              : await createStatus({ ...data, workspaceId });
        return normDict(row);
      }),

    update: publicQuery
      .input(
        z.object({
          kind: z.enum(["categories", "brands", "statuses"]),
          id: z.number().int().positive(),
          name: z.string().min(1).optional(),
          description: z.string().nullable().optional(),
          slug: z.string().optional(),
          color: z.string().optional(),
          bg: z.string().optional(),
        }),
      )
      .mutation(async ({ input }): Promise<DictEntry | undefined> => {
        const { kind, id, ...data } = input;
        const row =
          kind === "categories"
            ? await updateCategory(id, data)
            : kind === "brands"
              ? await updateBrand(id, data)
              : await updateStatus(id, data);
        return normDict(row);
      }),

    remove: publicQuery
      .input(
        z.object({
          kind: z.enum(["categories", "brands", "statuses"]),
          id: z.number().int().positive(),
        }),
      )
      .mutation(({ input }) => {
        if (input.kind === "categories") return deleteCategory(input.id);
        if (input.kind === "brands") return deleteBrand(input.id);
        return deleteStatus(input.id);
      }),
  }),
});
