import { z } from "zod";
import { createRouter, publicQuery } from "./middleware";
import { TRPCError } from "@trpc/server";
import { getDb } from "./queries/connection";
import { findHistory, appendHistory } from "./queries/history";
import { findItemById, updateItem } from "./queries/items";
import { getDefaultWorkspaceId, findStatuses, findStorages } from "./queries/catalog";
import { requireMe } from "./auth";

const dateRange = {
  dateFrom: z.date().optional(),
  dateTo: z.date().optional(),
};

export const historyRouter = createRouter({
  /** Перемещения (приём-передача, перемещения между складами/объектами). */
  movements: publicQuery
    .input(
      z.object({
        workspaceId: z.number().int().positive().optional(),
        itemId: z.number().int().positive().optional(),
        limit: z.number().int().min(1).max(500).default(200),
        ...dateRange,
      }),
    )
    .query(async ({ input }) => {
      const workspaceId = input.workspaceId ?? (await getDefaultWorkspaceId());
      return findHistory({
        workspaceId,
        types: ["move", "transfer_send", "transfer_receive"],
        dateFrom: input.dateFrom,
        dateTo: input.dateTo,
        itemId: input.itemId,
        limit: input.limit,
      });
    }),

  /** Количественные операции (списание/пополнение). */
  quantityOps: publicQuery
    .input(
      z.object({
        workspaceId: z.number().int().positive().optional(),
        itemId: z.number().int().positive().optional(),
        limit: z.number().int().min(1).max(500).default(200),
        ...dateRange,
      }),
    )
    .query(async ({ input }) => {
      const workspaceId = input.workspaceId ?? (await getDefaultWorkspaceId());
      return findHistory({
        workspaceId,
        types: ["write_off", "replenish"],
        dateFrom: input.dateFrom,
        dateTo: input.dateTo,
        itemId: input.itemId,
        limit: input.limit,
      });
    }),

  /** Все операции. */
  all: publicQuery
    .input(
      z.object({
        workspaceId: z.number().int().positive().optional(),
        itemId: z.number().int().positive().optional(),
        limit: z.number().int().min(1).max(500).default(200),
        ...dateRange,
      }),
    )
    .query(async ({ input }) => {
      const workspaceId = input.workspaceId ?? (await getDefaultWorkspaceId());
      return findHistory({
        workspaceId,
        dateFrom: input.dateFrom,
        dateTo: input.dateTo,
        itemId: input.itemId,
        limit: input.limit,
      });
    }),

  /** Списание количества (или статуса «Списан» для штучных). */
  writeOff: publicQuery
    .input(
      z.object({
        itemId: z.number().int().positive(),
        quantity: z.number().positive().optional(),
        comment: z.string().optional(),
        // Фото-подтверждение списания, если этого требует настройка группы (ТЗ §8).
        photoUrl: z.string().optional(),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      const me = await requireMe(ctx);
      const item = await findItemById(input.itemId);
      if (!item) throw new TRPCError({ code: "NOT_FOUND", message: "Инструмент не найден" });
      let delta: number | null = null;
      if (item.quantitative) {
        const qty = input.quantity ?? 1;
        const newQty = Math.max(0, (item.quantity ?? 0) - qty);
        await updateItem(item.id, { quantity: newQty });
        delta = -qty;
      } else {
        // Штучный инструмент — переводим в статус «Списан».
        const statuses = await findStatuses(item.workspaceId);
        const writtenOff = statuses.find((s) => s.slug === "written-off");
        if (writtenOff) await updateItem(item.id, { statusId: writtenOff.id });
      }
      await appendHistory(getDb(), {
        workspaceId: item.workspaceId,
        itemId: item.id,
        type: "write_off",
        actorUserId: me.id,
        fromLabel: item.title,
        quantityDelta: delta,
        comment: input.comment ?? "Списание",
      });
      return findItemById(item.id);
    }),

  /** Пополнение количественного инструмента/материала. */
  replenish: publicQuery
    .input(
      z.object({
        itemId: z.number().int().positive(),
        quantity: z.number().positive(),
        comment: z.string().optional(),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      const me = await requireMe(ctx);
      const item = await findItemById(input.itemId);
      if (!item) throw new TRPCError({ code: "NOT_FOUND", message: "Инструмент не найден" });
      if (!item.quantitative) {
        throw new TRPCError({ code: "BAD_REQUEST", message: "Инструмент не количественный" });
      }
      await updateItem(item.id, { quantity: (item.quantity ?? 0) + input.quantity });
      await appendHistory(getDb(), {
        workspaceId: item.workspaceId,
        itemId: item.id,
        type: "replenish",
        actorUserId: me.id,
        toLabel: item.title,
        quantityDelta: input.quantity,
        comment: input.comment ?? "Пополнение",
      });
      return findItemById(item.id);
    }),

  /** Перемещение между складами/объектами без смены ответственного. */
  move: publicQuery
    .input(
      z.object({
        itemId: z.number().int().positive(),
        toStorageId: z.number().int().positive().optional(),
        toBuildingSiteId: z.number().int().positive().nullable().optional(),
        comment: z.string().optional(),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      const me = await requireMe(ctx);
      const item = await findItemById(input.itemId);
      if (!item) throw new TRPCError({ code: "NOT_FOUND", message: "Инструмент не найден" });
      const storages = await findStorages(item.workspaceId);
      const fromStorage = storages.find((s) => s.id === item.storageId);
      const toStorage = input.toStorageId ? storages.find((s) => s.id === input.toStorageId) : null;
      await updateItem(item.id, {
        ...(input.toStorageId ? { storageId: input.toStorageId } : {}),
        ...(input.toBuildingSiteId !== undefined ? { buildingSiteId: input.toBuildingSiteId } : {}),
      });
      await appendHistory(getDb(), {
        workspaceId: item.workspaceId,
        itemId: item.id,
        type: "move",
        actorUserId: me.id,
        fromLabel: fromStorage?.name ?? null,
        toLabel: toStorage?.name ?? null,
        comment: input.comment ?? "Перемещение",
      });
      return findItemById(item.id);
    }),
});
