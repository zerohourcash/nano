import { z } from "zod";
import { createRouter, publicQuery } from "./middleware";
import { TRPCError } from "@trpc/server";
import { getDb } from "./queries/connection";
import {
  findInventorySessions,
  findInventorySessionById,
  createInventorySession,
  checkInventoryItem,
  uncheckInventoryItem,
  completeInventorySession,
} from "./queries/inventory";
import { appendHistory } from "./queries/history";
import { getDefaultWorkspaceId } from "./queries/catalog";
import { requireMe } from "./auth";

export const inventoryRouter = createRouter({
  sessions: publicQuery
    .input(z.object({ workspaceId: z.number().int().positive().optional() }).optional())
    .query(async ({ input }) => {
      const workspaceId = input?.workspaceId ?? (await getDefaultWorkspaceId());
      return findInventorySessions(workspaceId);
    }),

  byId: publicQuery
    .input(z.object({ id: z.number().int().positive() }))
    .query(async ({ input }) => {
      const session = await findInventorySessionById(input.id);
      if (!session) throw new TRPCError({ code: "NOT_FOUND", message: "Сессия не найдена" });
      return session;
    }),

  results: publicQuery
    .input(z.object({ sessionId: z.number().int().positive() }))
    .query(async ({ input }) => {
      const session = await findInventorySessionById(input.sessionId);
      if (!session) throw new TRPCError({ code: "NOT_FOUND", message: "Сессия не найдена" });
      return session.results;
    }),

  create: publicQuery
    .input(
      z.object({
        workspaceId: z.number().int().positive().optional(),
        storageId: z.number().int().positive().optional(),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      const me = await requireMe(ctx);
      const workspaceId = input.workspaceId ?? (await getDefaultWorkspaceId());
      return createInventorySession({ workspaceId, startedBy: me.id, storageId: input.storageId });
    }),

  checkItem: publicQuery
    .input(
      z.object({
        sessionId: z.number().int().positive(),
        itemId: z.number().int().positive(),
        actualQty: z.number().nonnegative().nullable().optional(),
        checked: z.boolean().default(true),
      }),
    )
    .mutation(async ({ input }) => {
      if (input.checked) {
        return checkInventoryItem(input.sessionId, input.itemId, input.actualQty ?? undefined);
      }
      return uncheckInventoryItem(input.sessionId, input.itemId);
    }),

  complete: publicQuery
    .input(z.object({ sessionId: z.number().int().positive() }))
    .mutation(async ({ ctx, input }) => {
      const me = await requireMe(ctx);
      const session = await findInventorySessionById(input.sessionId);
      if (!session) throw new TRPCError({ code: "NOT_FOUND", message: "Сессия не найдена" });
      const completed = await completeInventorySession(input.sessionId);
      // Фиксируем расхождения в журнале.
      for (const r of session.results) {
        if (r.checked && r.actualQty != null && r.expectedQty != null && r.actualQty !== r.expectedQty) {
          await appendHistory(getDb(), {
            workspaceId: session.workspaceId,
            itemId: r.itemId,
            type: "inventory",
            actorUserId: me.id,
            fromLabel: `ожидалось ${r.expectedQty}`,
            toLabel: `фактически ${r.actualQty}`,
            quantityDelta: r.actualQty - r.expectedQty,
            comment: `Инвентаризация ${session.number}`,
          });
        }
      }
      return completed;
    }),
});
