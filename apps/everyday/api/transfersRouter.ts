import { z } from "zod";
import { createRouter, publicQuery } from "./middleware";
import { TRPCError } from "@trpc/server";
import { getDb } from "./queries/connection";
import {
  findOutgoingTransfers,
  findIncomingTransfers,
  findTransferById,
  createTransfer,
  setTransferStatus,
  applyTransferToItem,
} from "./queries/transfers";
import { appendHistory } from "./queries/history";
import { findUserById } from "./queries/users";
import { requireMe } from "./auth";
import { findStatusBySlug } from "./queries/catalog";
import { findItemById } from "./queries/items";
import { createNotification } from "./queries/notifications";
import type { User } from "@db/schema";

async function takeItemForUser(me: User, itemId: number, comment?: string) {
  const item = await findItemById(itemId);
  if (!item) throw new TRPCError({ code: "NOT_FOUND", message: "Инструмент не найден" });
  if (item.status?.slug === "written-off") {
    throw new TRPCError({ code: "BAD_REQUEST", message: "Списанный инструмент нельзя взять" });
  }
  if (item.responsibleUserId === me.id) {
    throw new TRPCError({ code: "BAD_REQUEST", message: "Инструмент уже у вас" });
  }
  const fromUserId = item.responsibleUserId ?? item.storage?.responsibleUserId ?? me.id;
  const fromUser = fromUserId === me.id ? me : await findUserById(fromUserId);
  const inWork = await findStatusBySlug(item.workspaceId, "in-work");
  const transfer = await createTransfer({
    workspaceId: item.workspaceId,
    itemId: item.id,
    fromUserId,
    toUserId: me.id,
    toStorageId: item.storageId,
    buildingSiteId: item.buildingSiteId,
    comment: comment ?? "Выдача",
    noConfirmation: true,
    status: "pending",
  });
  if (!transfer) throw new TRPCError({ code: "INTERNAL_SERVER_ERROR", message: "Не удалось оформить выдачу" });
  await setTransferStatus(transfer.id, "accepted", { comment: comment ?? transfer.comment });
  await applyTransferToItem(item.id, {
    responsibleUserId: me.id,
    storageId: item.storageId,
    buildingSiteId: item.buildingSiteId,
    statusId: inWork?.id ?? item.statusId,
  });
  await appendHistory(getDb(), {
    workspaceId: item.workspaceId,
    itemId: item.id,
    type: "transfer_send",
    actorUserId: fromUserId,
    fromLabel: fromUser?.fullName ?? "Склад",
    toLabel: me.fullName,
    comment: `Выдача ${transfer.code ?? ""}: ${item.title}`,
  });
  await appendHistory(getDb(), {
    workspaceId: item.workspaceId,
    itemId: item.id,
    type: "transfer_receive",
    actorUserId: me.id,
    fromLabel: fromUser?.fullName ?? "Склад",
    toLabel: me.fullName,
    comment: `Получение ${transfer.code ?? ""}`,
  });
  if (fromUserId !== me.id) {
    await createNotification({
      userId: fromUserId,
      itemId: item.id,
      type: "transfer",
      title: "Инструмент выдан",
      text: `${me.fullName} забрал(а) ${item.title} (${item.internalId})`,
    });
  }
  return findItemById(item.id);
}

const prepareInput = z.object({
  itemId: z.number().int().positive(),
  toUserId: z.number().int().positive(),
  toStorageId: z.number().int().positive().nullable().optional(),
  buildingSiteId: z.number().int().positive().nullable().optional(),
  quantity: z.number().positive().nullable().optional(),
  comment: z.string().optional(),
  noConfirmation: z.boolean().default(false),
  asDraft: z.boolean().default(false),
});



export const transfersRouter = createRouter({
  /** Передачи, которые текущий пользователь отдаёт (draft/pending). */
  outgoing: publicQuery.query(async ({ ctx }) => {
    const me = await requireMe(ctx);
    return findOutgoingTransfers(me.id);
  }),

  /** Передачи, которые текущему пользователю нужно принять. */
  incoming: publicQuery.query(async ({ ctx }) => {
    const me = await requireMe(ctx);
    return findIncomingTransfers(me.id);
  }),

  byId: publicQuery
    .input(z.object({ id: z.number().int().positive() }))
    .query(async ({ input }) => {
      const t = await findTransferById(input.id);
      if (!t) throw new TRPCError({ code: "NOT_FOUND", message: "Передача не найдена" });
      return t;
    }),

  /** Добавить инструмент к передаче (создать передачу). */
  prepare: publicQuery.input(prepareInput).mutation(async ({ ctx, input }) => {
    const me = await requireMe(ctx);
    const item = await findItemById(input.itemId);
    if (!item) throw new TRPCError({ code: "NOT_FOUND", message: "Инструмент не найден" });
    const transfer = await createTransfer({
      workspaceId: item.workspaceId,
      itemId: input.itemId,
      fromUserId: me.id,
      toUserId: input.toUserId,
      toStorageId: input.toStorageId ?? null,
      buildingSiteId: input.buildingSiteId ?? null,
      quantity: input.quantity ?? null,
      comment: input.comment ?? null,
      noConfirmation: input.noConfirmation,
      status: input.asDraft ? "draft" : "pending",
    });
    if (transfer) {
      const toUser = await findUserById(input.toUserId);
      await appendHistory(getDb(), {
        workspaceId: transfer.workspaceId,
        itemId: transfer.itemId,
        type: "transfer_send",
        actorUserId: me.id,
        fromLabel: me.fullName,
        toLabel: toUser?.fullName ?? null,
        quantityDelta: input.quantity != null ? -Math.abs(input.quantity) : null,
        comment: `Передача ${transfer.code ?? ""} оформлена`.trim(),
      });
      if (!input.asDraft) {
        await createNotification({
          userId: input.toUserId,
          itemId: transfer.itemId,
          type: "transfer",
          title: "Ожидает приёма",
          text: `Передача ${transfer.code}: ${item.title} от ${me.fullName}`,
        });
      }
    }
    return transfer;
  }),

  accept: publicQuery
    .input(
      z.object({
        id: z.number().int().positive(),
        photoUrl: z.string().optional(),
        comment: z.string().optional(),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      const me = await requireMe(ctx);
      const t = await findTransferById(input.id);
      if (!t) throw new TRPCError({ code: "NOT_FOUND", message: "Передача не найдена" });
      if (t.status !== "pending" && t.status !== "draft") {
        throw new TRPCError({ code: "BAD_REQUEST", message: "Передача уже завершена" });
      }
      const updated = await setTransferStatus(input.id, "accepted", {
        photoUrl: input.photoUrl ?? null,
        comment: input.comment ?? t.comment,
      });
      // Переносим ответственность и склад на получателя.
      await applyTransferToItem(t.itemId, {
        responsibleUserId: t.toUserId,
        storageId: t.toStorageId ?? undefined,
        buildingSiteId: t.buildingSiteId ?? undefined,
      });
      await appendHistory(getDb(), {
        workspaceId: t.workspaceId,
        itemId: t.itemId,
        type: "transfer_receive",
        actorUserId: me.id,
        fromLabel: t.fromUser.fullName,
        toLabel: t.toUser.fullName,
        quantityDelta: t.quantity != null ? Math.abs(t.quantity) : null,
        comment: `Передача ${t.code ?? ""} принята`.trim(),
      });
      await createNotification({
        userId: t.fromUserId,
        itemId: t.itemId,
        type: "transfer",
        title: "Передача принята",
        text: `${t.toUser.fullName} принял(а) ${t.item.title} (${t.code})`,
      });
      return updated;
    }),

  reject: publicQuery
    .input(
      z.object({
        id: z.number().int().positive(),
        comment: z.string().optional(),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      const me = await requireMe(ctx);
      const t = await findTransferById(input.id);
      if (!t) throw new TRPCError({ code: "NOT_FOUND", message: "Передача не найдена" });
      if (t.status !== "pending" && t.status !== "draft") {
        throw new TRPCError({ code: "BAD_REQUEST", message: "Передача уже завершена" });
      }
      const updated = await setTransferStatus(input.id, "rejected", {
        comment: input.comment ?? t.comment,
      });
      await appendHistory(getDb(), {
        workspaceId: t.workspaceId,
        itemId: t.itemId,
        type: "transfer_receive",
        actorUserId: me.id,
        fromLabel: t.fromUser.fullName,
        toLabel: t.toUser.fullName,
        comment: `Передача ${t.code ?? ""} отклонена${input.comment ? ": " + input.comment : ""}`,
      });
      await createNotification({
        userId: t.fromUserId,
        itemId: t.itemId,
        type: "transfer",
        title: "Передача отклонена",
        text: `${t.toUser.fullName} отклонил(а) ${t.item.title} (${t.code})`,
      });
      return updated;
    }),

  /** Принять все входящие передачи разом. */
  acceptAll: publicQuery.mutation(async ({ ctx }) => {
    const me = await requireMe(ctx);
    const incoming = await findIncomingTransfers(me.id);
    const accepted = [];
    for (const t of incoming) {
      const updated = await setTransferStatus(t.id, "accepted");
      await applyTransferToItem(t.itemId, {
        responsibleUserId: t.toUserId,
        storageId: t.toStorageId ?? undefined,
        buildingSiteId: t.buildingSiteId ?? undefined,
      });
      await appendHistory(getDb(), {
        workspaceId: t.workspaceId,
        itemId: t.itemId,
        type: "transfer_receive",
        actorUserId: me.id,
        fromLabel: t.fromUser.fullName,
        toLabel: t.toUser.fullName,
        quantityDelta: t.quantity != null ? Math.abs(t.quantity) : null,
        comment: `Передача ${t.code ?? ""} принята (массовый приём)`,
      });
      await createNotification({
        userId: t.fromUserId,
        itemId: t.itemId,
        type: "transfer",
        title: "Передача принята",
        text: `${t.toUser.fullName} принял(а) ${t.item.title} (${t.code})`,
      });
      accepted.push(updated);
    }
    return { acceptedCount: accepted.length, accepted };
  }),

  take: publicQuery
    .input(
      z.object({
        itemId: z.number().int().positive(),
        comment: z.string().optional(),
        dueAt: z.string().optional(),
        photoUrl: z.string().optional(),
        quantity: z.number().positive().optional(),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      const me = await requireMe(ctx);
      return takeItemForUser(me, input.itemId, input.comment);
    }),

  takeMany: publicQuery
    .input(
      z.object({
        itemIds: z.array(z.number().int().positive()).min(1).max(50),
        comment: z.string().optional(),
        dueAt: z.string().optional(),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      const me = await requireMe(ctx);
      const taken: number[] = [];
      const failed: Array<{ itemId: number; message: string }> = [];
      for (const itemId of input.itemIds) {
        try {
          await takeItemForUser(me, itemId, input.comment);
          taken.push(itemId);
        } catch (err) {
          const message = err instanceof TRPCError ? err.message : "Не удалось взять";
          failed.push({ itemId, message });
        }
      }
      return { takenCount: taken.length, taken, failed };
    }),

  /** Вернуть предмет на склад. */
  returnItem: publicQuery
    .input(
      z.object({
        itemId: z.number().int().positive(),
        storageId: z.number().int().positive().optional(),
        comment: z.string().optional(),
        quantity: z.number().positive().optional(),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      const me = await requireMe(ctx);
      const item = await findItemById(input.itemId);
      if (!item) throw new TRPCError({ code: "NOT_FOUND", message: "Инструмент не найден" });
      if (item.responsibleUserId && item.responsibleUserId !== me.id) {
        throw new TRPCError({
          code: "BAD_REQUEST",
          message: `Инструмент на ответственном: ${item.responsible?.fullName ?? "другой сотрудник"}`,
        });
      }
      if (!item.responsibleUserId) {
        throw new TRPCError({ code: "BAD_REQUEST", message: "Инструмент уже на складе" });
      }
      const keeperId = item.storage?.responsibleUserId ?? me.id;
      const inStock = await findStatusBySlug(item.workspaceId, "in-stock");
      const transfer = await createTransfer({
        workspaceId: item.workspaceId,
        itemId: item.id,
        fromUserId: me.id,
        toUserId: keeperId,
        toStorageId: input.storageId ?? item.storageId,
        comment: input.comment ?? "Возврат на склад",
        noConfirmation: true,
        status: "pending",
      });
      if (transfer) await setTransferStatus(transfer.id, "accepted", { comment: input.comment ?? transfer.comment });
      await applyTransferToItem(item.id, {
        responsibleUserId: null,
        storageId: input.storageId ?? item.storageId,
        buildingSiteId: null,
        statusId: inStock?.id ?? item.statusId,
      });
      await appendHistory(getDb(), {
        workspaceId: item.workspaceId,
        itemId: item.id,
        type: "transfer_send",
        actorUserId: me.id,
        fromLabel: me.fullName,
        toLabel: "Склад",
        comment: `Возврат ${item.internalId} на склад`,
      });
      await appendHistory(getDb(), {
        workspaceId: item.workspaceId,
        itemId: item.id,
        type: "transfer_receive",
        actorUserId: keeperId,
        fromLabel: me.fullName,
        toLabel: "Склад",
        comment: `Принят на склад ${item.internalId}`,
      });
      return findItemById(item.id);
    }),
});
