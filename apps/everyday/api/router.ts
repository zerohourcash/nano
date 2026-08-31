import { createRouter, publicQuery } from "./middleware";
import { itemsRouter } from "./itemsRouter";
import { transfersRouter } from "./transfersRouter";
import { historyRouter } from "./historyRouter";
import { inventoryRouter } from "./inventoryRouter";
import { notificationsRouter } from "./notificationsRouter";
import { reportsRouter } from "./reportsRouter";
import { adminRouter } from "./adminRouter";
import { profileRouter } from "./profileRouter";
import { metaRouter } from "./metaRouter";
import { authRouter } from "./authRouter";
import { chatRouter } from "./chatRouter";
import { backupRouter, syncRouter } from "./syncRouter";

export const appRouter = createRouter({
  ping: publicQuery.query(() => ({ ok: true, ts: Date.now() })),
  auth: authRouter,

  items: itemsRouter,
  transfers: transfersRouter,
  history: historyRouter,
  inventory: inventoryRouter,
  notifications: notificationsRouter,
  reports: reportsRouter,
  admin: adminRouter,
  profile: profileRouter,
  meta: metaRouter,
  chat: chatRouter,
  sync: syncRouter,
  backup: backupRouter,
});

export type AppRouter = typeof appRouter;
