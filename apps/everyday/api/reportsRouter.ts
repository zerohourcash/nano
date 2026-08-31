import { z } from "zod";
import { createRouter, publicQuery } from "./middleware";
import {
  reportByUsers,
  reportQuantityTransactions,
  reportAllItems,
} from "./queries/notifications";
import { getDefaultWorkspaceId } from "./queries/catalog";

export const reportsRouter = createRouter({
  /** Отчёт по ответственным пользователям. */
  byUsers: publicQuery
    .input(z.object({ workspaceId: z.number().int().positive().optional() }).optional())
    .query(async ({ input }) => {
      const workspaceId = input?.workspaceId ?? (await getDefaultWorkspaceId());
      return reportByUsers(workspaceId);
    }),

  /** Поступление/списание за диапазон дат. */
  quantityTransactions: publicQuery
    .input(
      z.object({
        workspaceId: z.number().int().positive().optional(),
        dateFrom: z.date().optional(),
        dateTo: z.date().optional(),
      }),
    )
    .query(async ({ input }) => {
      const workspaceId = input.workspaceId ?? (await getDefaultWorkspaceId());
      return reportQuantityTransactions(workspaceId, input.dateFrom, input.dateTo);
    }),

  /** Все инструменты (полный реестр имущества). */
  allItems: publicQuery
    .input(z.object({ workspaceId: z.number().int().positive().optional() }).optional())
    .query(async ({ input }) => {
      const workspaceId = input?.workspaceId ?? (await getDefaultWorkspaceId());
      return reportAllItems(workspaceId);
    }),
});
