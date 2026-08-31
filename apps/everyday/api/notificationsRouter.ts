import { z } from "zod";
import { createRouter, publicQuery } from "./middleware";
import {
  findNotifications,
  unreadCount,
  markNotificationRead,
  markAllNotificationsRead,
} from "./queries/notifications";
import { requireMe } from "./auth";

export const notificationsRouter = createRouter({
  list: publicQuery.query(async ({ ctx }) => {
    const me = await requireMe(ctx);
    return findNotifications(me.id);
  }),

  unreadCount: publicQuery.query(async ({ ctx }) => {
    const me = await requireMe(ctx);
    return { count: await unreadCount(me.id) };
  }),

  markRead: publicQuery
    .input(z.object({ id: z.number().int().positive() }))
    .mutation(({ input }) => markNotificationRead(input.id)),

  markAllRead: publicQuery.mutation(async ({ ctx }) => {
    const me = await requireMe(ctx);
    return markAllNotificationsRead(me.id);
  }),
});
