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
      guid: string;
      workspaceId: number;
      userId: number;
      text: string;
      createdAt: string;
      ledgerHash: string | null;
      ledgerVerified: boolean;
      attachments: Array<{ name: string; url: string; mime: string; sha256: string }>;
      user: { id: number; fullName: string; avatarUrl: string | null } | null;
    }>),

  send: publicQuery
    .input(
      z.object({
        text: z.string().min(1),
        workspaceId: z.number().int().positive().optional(),
        workspaceGuid: z.string().uuid().optional(),
        messageGuid: z.string().uuid().optional(),
        attachments: z.array(z.object({
          name: z.string().min(1).max(200),
          url: z.string().regex(/^cas:[a-f0-9]{64}$/i),
          mime: z.string().min(1).max(100),
        })).max(10).default([]),
      }),
    )
    .mutation(async ({ input }) => ({
      id: 0,
      guid: "",
      workspaceId: 1,
      userId: 0,
      text: input.text,
      createdAt: new Date().toISOString(),
      ledgerHash: "",
      ledgerVerified: true,
      attachments: input.attachments.map(attachment => ({ ...attachment, sha256: attachment.url.slice(4) })),
      user: null as { id: number; fullName: string; avatarUrl: string | null } | null,
    })),
});

void chatUser;
