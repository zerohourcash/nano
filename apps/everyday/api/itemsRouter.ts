import { z } from "zod";
import { createRouter, publicQuery } from "./middleware";
import { TRPCError } from "@trpc/server";
import { getDb } from "./queries/connection";
import {
  findItems,
  findItemById,
  findItemByCode,
  countItems,
  nextInternalId,
  createItem,
  updateItem,
  deleteItem,
  addItemPhoto,
  addItemComment,
} from "./queries/items";
import { findItemHistory, appendHistory } from "./queries/history";
import { requireMe } from "./auth";
import { getDefaultWorkspaceId, findWorkspaceById } from "./queries/catalog";
import { createNotification } from "./queries/notifications";

const listInput = z.object({
  workspaceId: z.number().int().positive().optional(),
  search: z.string().optional(),
  userId: z.number().int().positive().optional(),
  buildingSiteId: z.number().int().positive().optional(),
  storageId: z.number().int().positive().optional(),
  categoryId: z.number().int().positive().optional(),
  brandId: z.number().int().positive().optional(),
  statusId: z.number().int().positive().optional(),
  organizationNodeId: z.number().int().positive().optional(),
  hasQr: z.boolean().optional(),
  onlyMine: z.boolean().optional(),
  page: z.number().int().min(1).default(1),
  limit: z.number().int().min(1).max(500).default(20),
  sort: z
    .enum(["createdAt_desc", "createdAt_asc", "title_asc", "title_desc", "internalId_asc"])
    .default("createdAt_desc"),
});

export const itemsRouter = createRouter({
  list: publicQuery.input(listInput).query(async ({ ctx, input }) => {
    const workspaceId = input.workspaceId ?? (await getDefaultWorkspaceId());
    const me = input.onlyMine ? await requireMe(ctx) : null;
    const result = await findItems({
      workspaceId,
      search: input.search,
      userId: input.userId,
      buildingSiteId: input.buildingSiteId,
      storageId: input.storageId,
      categoryId: input.categoryId,
      brandId: input.brandId,
      statusId: input.statusId,
      organizationNodeId: input.organizationNodeId,
      hasQr: input.hasQr,
      onlyMineUserId: me?.id,
      page: input.page,
      limit: input.limit,
      sort: input.sort,
    });
    return {
      ...result,
      rows: result.rows.map((item) => ({ ...item, guid: null as string | null })),
      total: await countItems(workspaceId),
    };
  }),

  byId: publicQuery
    .input(z.object({ id: z.number().int().positive() }))
    .query(async ({ input }) => {
      const item = await findItemById(input.id);
      if (!item) throw new TRPCError({ code: "NOT_FOUND", message: "Инструмент не найден" });
      const history = await findItemHistory(input.id);
      // Rust node supplies the globally stable GUID used by canonical QR tags.
      // The legacy TypeScript fallback has no GUID column and keeps old tags.
      return { ...item, guid: null as string | null, history };
    }),

  byCode: publicQuery
    .input(z.object({ code: z.string().min(1) }))
    .query(async ({ input }) => {
      const item = await findItemByCode(input.code);
      if (!item) throw new TRPCError({ code: "NOT_FOUND", message: "Инструмент с таким QR/номером не найден" });
      return {
        ...item,
        qrVerification: {
          version: 1,
          authenticity: 'legacy-unverified' as 'legacy-unverified' | 'trusted-node',
        },
      };
    }),

  qrLabel: publicQuery
    .input(z.object({ itemId: z.number().int().positive() }))
    .query(async ({ input }) => {
      const item = await findItemById(input.itemId);
      if (!item) throw new TRPCError({ code: "NOT_FOUND", message: "Инструмент не найден" });
      const guid = (item as typeof item & { guid?: string | null }).guid;
      return {
        label: guid ? `everyday:item:${guid}` : (item.qrCode ?? item.internalId),
        version: 1,
        algorithm: 'legacy',
        cloneResistant: false,
      };
    }),

  nextInternalId: publicQuery
    .input(z.object({ workspaceId: z.number().int().positive().optional() }).optional())
    .query(async ({ input }) => {
      const workspaceId = input?.workspaceId ?? (await getDefaultWorkspaceId());
      const ws = await findWorkspaceById(workspaceId);
      return nextInternalId(workspaceId, ws?.internalIdPrefix ?? "ВН-");
    }),

  create: publicQuery
    .input(
      z.object({
        workspaceId: z.number().int().positive().optional(),
        internalId: z.string().optional(),
        title: z.string().min(1),
        categoryId: z.number().int().positive().optional(),
        brandId: z.number().int().positive().optional(),
        statusId: z.number().int().positive().optional(),
        responsibleUserId: z.number().int().positive().nullable().optional(),
        buildingSiteId: z.number().int().positive().nullable().optional(),
        storageId: z.number().int().positive().nullable().optional(),
        organizationNodeId: z.number().int().positive().nullable().optional(),
        serialNumber: z.string().optional(),
        cost: z.number().nonnegative().optional(),
        quantitative: z.boolean().default(false),
        quantity: z.number().nonnegative().optional(),
        unit: z.string().optional(),
        comment: z.string().optional(),
        qrCode: z.string().optional(),
        notifyDate: z.date().optional(),
        sourceSystem: z.string().max(64).optional(),
        externalId: z.string().max(128).optional(),
        metadata: z.record(z.string(), z.unknown()).optional(),
        // Строка — совместимость со старыми клиентами; объект — оригинал
        // вместе с уменьшенной копией (ТЗ §5).
        photos: z
          .array(z.union([z.string(), z.object({ url: z.string(), thumbUrl: z.string() })]))
          .optional(),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      const workspaceId = input.workspaceId ?? (await getDefaultWorkspaceId());
      const ws = await findWorkspaceById(workspaceId);
      const internalId =
        input.internalId ?? (await nextInternalId(workspaceId, ws?.internalIdPrefix ?? "ВН-"));
      const { workspaceId: _ws, internalId: _iid, ...rest } = input;
      void _ws;
      void _iid;
      const item = await createItem({
        ...rest,
        workspaceId,
        internalId,
        qrCode: rest.qrCode ?? internalId,
      });
      const me = await requireMe(ctx);
      if (item && me) {
        await appendHistory(getDb(), {
          workspaceId,
          itemId: item.id,
          type: "create",
          actorUserId: me.id,
          toLabel: item.title,
          comment: "Инструмент добавлен в каталог",
        });
        await createNotification({
          userId: me.id,
          itemId: item.id,
          type: "system",
          title: "Инструмент создан",
          text: `${item.internalId} ${item.title} добавлен в каталог`,
        });
      }
      return item;
    }),

  update: publicQuery
    .input(
      z.object({
        id: z.number().int().positive(),
        title: z.string().min(1).optional(),
        categoryId: z.number().int().positive().nullable().optional(),
        brandId: z.number().int().positive().nullable().optional(),
        statusId: z.number().int().positive().nullable().optional(),
        responsibleUserId: z.number().int().positive().nullable().optional(),
        buildingSiteId: z.number().int().positive().nullable().optional(),
        storageId: z.number().int().positive().nullable().optional(),
        organizationNodeId: z.number().int().positive().nullable().optional(),
        serialNumber: z.string().nullable().optional(),
        cost: z.number().nonnegative().nullable().optional(),
        quantitative: z.boolean().optional(),
        quantity: z.number().nonnegative().nullable().optional(),
        unit: z.string().nullable().optional(),
        comment: z.string().nullable().optional(),
        qrCode: z.string().nullable().optional(),
        notifyDate: z.date().nullable().optional(),
        calibratedUntil: z.string().nullable().optional(),
        minQuantity: z.number().nullable().optional(),
        sourceSystem: z.string().max(64).nullable().optional(),
        externalId: z.string().max(128).nullable().optional(),
        metadata: z.record(z.string(), z.unknown()).nullable().optional(),
        // Причина перевода в «неисправен» / «на ремонте» / «списан» (ТЗ §8).
        reason: z.string().optional(),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      const { id, reason, ...data } = input;
      void reason; // причину пишет Rust-узел в подписанный журнал
      const before = await findItemById(id);
      if (!before) throw new TRPCError({ code: "NOT_FOUND", message: "Инструмент не найден" });
      const item = await updateItem(id, data);
      const me = await requireMe(ctx);
      if (item && me) {
        await appendHistory(getDb(), {
          workspaceId: item.workspaceId,
          itemId: id,
          type: "update",
          actorUserId: me.id,
          fromLabel: before.title,
          toLabel: item.title,
          comment: "Данные инструмента обновлены",
        });
      }
      return item;
    }),

  remove: publicQuery
    .input(z.object({ id: z.number().int().positive() }))
    .mutation(({ input }) => deleteItem(input.id)),

  addPhoto: publicQuery
    .input(
      z.object({
        itemId: z.number().int().positive(),
        itemGuid: z.string().uuid(),
        photoGuid: z.string().uuid(),
        url: z.string().min(1),
        thumbUrl: z.string().min(1).optional(),
        isTitle: z.boolean().default(false),
      }),
    )
    .mutation(({ input }) => addItemPhoto(input.itemId, input.url, input.isTitle)),

  addDocument: publicQuery
    .input(z.object({
      itemId: z.number().int().positive(),
      itemGuid: z.string().uuid(),
      documentGuid: z.string().uuid(),
      name: z.string().min(1).max(200),
      url: z.string().regex(/^cas:[a-f0-9]{64}$/i),
      mime: z.string().min(1).max(100),
      accessLevel: z.enum(['members', 'accounting', 'managers']).default('members'),
    }))
    .mutation(async ({ input }) => ({ id: 0, guid: input.documentGuid, ...input, sha256: input.url.slice(4) })),

  addComment: publicQuery
    .input(
      z.object({
        itemId: z.number().int().positive(),
        userId: z.number().int().positive().optional(),
        text: z.string().min(1),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      const me = input.userId ? null : await requireMe(ctx);
      const userId = input.userId ?? me?.id;
      if (!userId) throw new TRPCError({ code: "BAD_REQUEST", message: "Нет пользователя" });
      return addItemComment(input.itemId, userId, input.text);
    }),

  reportFault: publicQuery
    .input(
      z.object({
        itemId: z.number().int().positive(),
        description: z.string().min(1),
        severity: z.enum(["low", "medium", "high"]).default("medium"),
        photoUrl: z.string().optional(),
      }),
    )
    .mutation(async () => ({ id: 0, itemId: 0, status: "open" })),

  faults: publicQuery
    .input(z.object({ itemId: z.number().int().positive().optional(), workspaceId: z.number().int().positive().optional() }).optional())
    .query(async () => [] as Array<{
      id: number;
      guid: string | null;
      itemId: number;
      workspaceId: number;
      authorId: number;
      severity: string;
      description: string;
      photoUrl: string | null;
      status: string;
      resolution: string | null;
      resolverId: number | null;
      createdAt: string;
      resolvedAt: string | null;
      author: { id: number; fullName: string } | null;
    }>),

  resolveFault: publicQuery
    .input(
      z.object({
        id: z.number().int().positive(),
        status: z.string().optional(),
        comment: z.string().optional(),
      }),
    )
    .mutation(async ({ input }) => ({ ok: true, id: input.id, status: input.status ?? "resolved" })),

  requestChange: publicQuery
    .input(
      z.object({
        itemId: z.number().int().positive(),
        payload: z.any().optional(),
        comment: z.string().optional(),
      }),
    )
    .mutation(async () => ({ id: 0, status: "pending" })),

  changeRequests: publicQuery
    .input(z.object({ workspaceId: z.number().int().positive().optional() }).optional())
    .query(async () => [] as Array<{
      id: number;
      guid: string | null;
      itemId: number;
      workspaceId: number;
      authorId: number;
      payload: Record<string, unknown>;
      comment: string | null;
      status: string;
      reason: string | null;
      decidedBy: number | null;
      createdAt: string;
      decidedAt: string | null;
      author: { id: number; fullName: string } | null;
      item: { id: number; title: string; internalId: string } | null;
      // Готовое сравнение «было / предлагается» (ТЗ §4): узел разворачивает
      // идентификаторы справочников в названия.
      changes: Array<{
        field: string;
        label: string;
        before: string | null;
        after: string | null;
      }>;
    }>),

  decideChange: publicQuery
    .input(
      z.object({
        id: z.number().int().positive(),
        accept: z.boolean(),
        reason: z.string().optional(),
      }),
    )
    .mutation(async ({ input }) => ({ ok: true, status: input.accept ? "accepted" : "rejected" })),
});
