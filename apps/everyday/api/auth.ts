import { TRPCError } from "@trpc/server";
import type { TrpcContext } from "./context";
import { findUserById } from "./queries/users";

export async function requireMe(ctx: TrpcContext) {
  if (!ctx.userId) {
    throw new TRPCError({ code: "UNAUTHORIZED", message: "Войдите в систему" });
  }
  const me = await findUserById(ctx.userId);
  if (!me || me.status === "disabled") {
    throw new TRPCError({ code: "UNAUTHORIZED", message: "Пользователь не найден" });
  }
  return me;
}

export async function optionalMe(ctx: TrpcContext) {
  if (!ctx.userId) return null;
  return findUserById(ctx.userId);
}
