import { z } from "zod";
import { createRouter, publicQuery } from "./middleware";
import { updateUser, workspacesOfUser } from "./queries/users";
import { requireMe } from "./auth";
import { publicUser } from "./lib/password";
import { TRPCError } from "@trpc/server";

export const profileRouter = createRouter({
  get: publicQuery.query(async ({ ctx }) => {
    const me = await requireMe(ctx);
    const workspaces = await workspacesOfUser(me.id);
    return { ...publicUser(me), workspaces };
  }),

  update: publicQuery
    .input(
      z.object({
        fullName: z.string().min(1).optional(),
        position: z.string().nullable().optional(),
        phone: z.string().min(5).optional(),
        avatarUrl: z.string().nullable().optional(),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      const me = await requireMe(ctx);
      return updateUser(me.id, input);
    }),

  changePassword: publicQuery
    .input(
      z.object({
        currentPassword: z.string().min(1),
        newPassword: z.string().min(12).max(128),
      }),
    )
    .mutation(async () => {
      throw new TRPCError({ code: "PRECONDITION_FAILED", message: "Смена пароля доступна на автономном Rust-узле" });
    }),

  // Production uses the Rust node. The legacy adapter fails closed instead of
  // pretending that destructive local-only actions succeeded.
  leaveWorkspace: publicQuery
    .input(z.object({ workspaceId: z.number().int().positive() }))
    .mutation(async () => {
      throw new TRPCError({ code: "PRECONDITION_FAILED", message: "Требуется автономный Rust-узел" });
    }),

  deleteAccount: publicQuery
    .input(z.object({ currentPassword: z.string().min(1) }))
    .mutation(async () => {
      throw new TRPCError({ code: "PRECONDITION_FAILED", message: "Требуется автономный Rust-узел" });
    }),
});
