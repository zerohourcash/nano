import { createRouter, publicQuery } from "./middleware";
import { requireMe } from "./auth";
import { transferCounts } from "./queries/transfers";
import { findWorkspaces } from "./queries/catalog";
import { publicUser } from "./lib/password";

export const metaRouter = createRouter({
  currentUser: publicQuery.query(async ({ ctx }) => publicUser(await requireMe(ctx))),

  transferCounts: publicQuery.query(async ({ ctx }) => {
    const me = await requireMe(ctx);
    return transferCounts(me.id);
  }),

  /** Список рабочих пространств. */
  workspaces: publicQuery.query(() => findWorkspaces()),
});
