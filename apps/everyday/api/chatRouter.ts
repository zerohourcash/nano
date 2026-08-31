import { z } from "zod";
import { createRouter, publicQuery } from "./middleware";

const chatUser = z
  .object({
    id: z.number(),
    fullName: z.string(),
    avatarUrl: z.string().nullable().optional(),
  })
  .nullable();

export const chatRouter = createRouter({
  list: publicQuery
    .input(z.object({ workspaceId: z.number().int().positive().optional() }).optional())
    .query(async () => [] as Array<{
      id: number;
      workspaceId: number;
      userId: number;
      text: string;
      createdAt: string;
      user: { id: number; fullName: string; avatarUrl: string | null } | null;
    }>),

  send: publicQuery
    .input(
      z.object({
        text: z.string().min(1),
        workspaceId: z.number().int().positive().optional(),
      }),
    )
    .mutation(async ({ input }) => ({
      id: 0,
      workspaceId: 1,
      userId: 0,
      text: input.text,
      createdAt: new Date().toISOString(),
      user: null as { id: number; fullName: string; avatarUrl: string | null } | null,
    })),
});

void chatUser;
