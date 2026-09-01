import { z } from "zod";
import { TRPCError } from "@trpc/server";
import { createRouter, publicQuery } from "./middleware";
import { findUserById, findUserByPhone, findUsers, createUser, updateUser } from "./queries/users";
import { createWorkspace, getDefaultWorkspaceId } from "./queries/catalog";
import { requireMe } from "./auth";
import { hashPassword, verifyPassword, publicUser } from "./lib/password";
import type { RoleRights } from "@db/schema";

const OWNER_RIGHTS: RoleRights = {
  viewItems: true,
  createItems: true,
  editItems: true,
  deleteItems: true,
  transferItems: true,
  acceptTransfers: true,
  writeOff: true,
  replenish: true,
  inventory: true,
  viewHistory: true,
  viewReports: true,
  manageUsers: true,
  manageWorkspaces: true,
  manageStorages: true,
  manageSites: true,
  manageDictionaries: true,
};

function setUserCookie(headers: Headers, userId: number | null) {
  if (userId) {
    headers.append("Set-Cookie", `mk_user=${userId}; Path=/; SameSite=Lax; Max-Age=2592000`);
  } else {
    headers.append("Set-Cookie", "mk_user=; Path=/; SameSite=Lax; Max-Age=0");
  }
}

function timezoneFromLabel(label: string): string {
  if (label.includes("Калининград")) return "Europe/Kaliningrad";
  if (label.includes("Самара")) return "Europe/Samara";
  if (label.includes("Екатеринбург")) return "Asia/Yekaterinburg";
  if (label.includes("Новосибирск")) return "Asia/Novosibirsk";
  if (label.includes("Владивосток")) return "Asia/Vladivostok";
  return "Europe/Moscow";
}

export const authRouter = createRouter({
  directory: publicQuery.query(async () => {
    const users = await findUsers();
    return users
      .filter((u) => u.status !== "disabled")
      .map((u) => ({ ...publicUser(u), hasPassword: Boolean(u.passwordHash) }));
  }),

  login: publicQuery
    .input(
      z.object({
        phone: z.string().optional(),
        userId: z.number().int().positive().optional(),
        password: z.string().optional(),
      }),
    )
    .mutation(async ({ input, ctx }) => {
      let user = input.userId ? await findUserById(input.userId) : null;
      if (!user && input.phone) user = await findUserByPhone(input.phone);
      if (!user) {
        throw new TRPCError({
          code: "UNAUTHORIZED",
          message: "Аккаунт не найден. Зарегистрируйтесь или выберите сотрудника из списка.",
        });
      }
      if (user.status === "disabled") {
        throw new TRPCError({ code: "UNAUTHORIZED", message: "Аккаунт заблокирован" });
      }
      if (user.passwordHash) {
        if (!input.password || !verifyPassword(input.password, user.passwordHash)) {
          throw new TRPCError({ code: "UNAUTHORIZED", message: "Неверный пароль" });
        }
      }
      if (user.status === "invited") {
        await updateUser(user.id, { status: "active" });
        user = await findUserById(user.id);
      }
      setUserCookie(ctx.resHeaders, user!.id);
      return publicUser(user!);
    }),

  register: publicQuery
    .input(
      z.object({
        fullName: z.string().min(2),
        phone: z.string().min(5),
        password: z.string().min(12).max(128),
        workspaceName: z.string().min(1),
        timezone: z.string().optional(),
        syncUrl: z.string().optional(),
      }),
    )
    .mutation(async ({ input, ctx }) => {
      const existing = await findUserByPhone(input.phone);
      if (existing) {
        throw new TRPCError({
          code: "CONFLICT",
          message: "Этот телефон уже зарегистрирован. Войдите с тем же номером и паролем.",
        });
      }
      let workspaceId: number;
      try {
        const ws = await createWorkspace({
          name: input.workspaceName.trim(),
          timezone: timezoneFromLabel(input.timezone ?? "Москва"),
          internalIdPrefix: "ВН-",
          comment: "Создано при регистрации",
        });
        workspaceId = ws?.id ?? (await getDefaultWorkspaceId());
      } catch {
        workspaceId = await getDefaultWorkspaceId();
      }
      let created;
      try {
        created = await createUser({
          fullName: input.fullName.trim(),
          phone: input.phone.trim(),
          passwordHash: hashPassword(input.password),
          roleRights: OWNER_RIGHTS,
          workspaceIds: [workspaceId],
        });
      } catch (err) {
        const message = err instanceof Error ? err.message : "";
        if (message.toLowerCase().includes("unique")) {
          throw new TRPCError({
            code: "CONFLICT",
            message: "Этот телефон уже зарегистрирован. Войдите с тем же номером и паролем.",
          });
        }
        throw err;
      }
      if (!created) {
        throw new TRPCError({ code: "INTERNAL_SERVER_ERROR", message: "Не удалось создать аккаунт" });
      }
      setUserCookie(ctx.resHeaders, created.id);
      return publicUser(created);
    }),

  logout: publicQuery.mutation(async ({ ctx }) => {
    setUserCookie(ctx.resHeaders, null);
    return { ok: true };
  }),

  me: publicQuery.query(async ({ ctx }) => {
    try {
      return publicUser(await requireMe(ctx));
    } catch {
      return null;
    }
  }),

  registerDevice: publicQuery
    .input(z.object({
      deviceId: z.string().min(16).max(100),
      name: z.string().min(1).max(100),
      publicKey: z.string().min(40).max(100),
    }))
    .mutation(({ input }) => ({ ...input, revoked: false })),

  devices: publicQuery.query(async () => [] as Array<{
    deviceId: string
    name: string
    publicKey: string
    createdAt: string
    lastSeenAt: string | null
    revoked: boolean
  }>),

  revokeDevice: publicQuery
    .input(z.object({ deviceId: z.string().min(16).max(100) }))
    .mutation(() => ({ ok: true })),

  options: publicQuery.query(async () => ({
    registrationOpen: false,
    bootstrap: false,
    demoLogin: false,
    androidTestBuild: null as null | {
      url: string
      sha256: string
      sizeBytes: number
      debug: boolean
    },
    desktopTestBuild: null as null | {
      url: string
      sha256: string
      sizeBytes: number
      debug: boolean
    },
  })),

  inviteInfo: publicQuery
    .input(z.object({ token: z.string().min(1) }))
    .query(async () => ({
      workspace: {
        id: 1,
        name: "",
        timezone: "Europe/Moscow",
        internalIdPrefix: "ВН-",
        comment: null as string | null,
        createdAt: new Date(),
      },
      role: "member",
      token: "",
      expiresAt: null as string | null,
    })),

  join: publicQuery
    .input(z.object({ token: z.string().min(1) }))
    .mutation(async () => ({ id: 1, name: "", timezone: "Europe/Moscow", internalIdPrefix: "ВН-", comment: null as string | null, createdAt: new Date() })),

  joinRegister: publicQuery
    .input(
      z.object({
        token: z.string().min(1),
        fullName: z.string().min(2),
        phone: z.string().min(5),
        password: z.string().min(12).max(128),
      }),
    )
    .mutation(async ({ input, ctx }) => {
      const existing = await findUserByPhone(input.phone);
      if (existing) {
        setUserCookie(ctx.resHeaders, existing.id);
        return publicUser(existing);
      }
      const created = await createUser({
        fullName: input.fullName,
        phone: input.phone,
        passwordHash: hashPassword(input.password),
      });
      if (!created) throw new TRPCError({ code: "INTERNAL_SERVER_ERROR", message: "Не удалось создать аккаунт" });
      setUserCookie(ctx.resHeaders, created.id);
      return publicUser(created);
    }),
});
