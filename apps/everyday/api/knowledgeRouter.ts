import { z } from 'zod'
import { createRouter, publicQuery } from './middleware'

const attachment = z.object({ name: z.string().min(1).max(200), url: z.string().min(1), mime: z.string().optional() })

export const knowledgeRouter = createRouter({
  list: publicQuery.input(z.object({ workspaceId: z.number().int().positive() })).query(async () => [] as Array<Record<string, unknown>>),
  bySlug: publicQuery.input(z.object({ workspaceId: z.number().int().positive(), slug: z.string().min(1) })).query(async () => null as Record<string, unknown> | null),
  save: publicQuery.input(z.object({
    workspaceId: z.number().int().positive(),
    workspaceGuid: z.string().uuid().optional(),
    slug: z.string().min(1).max(120),
    title: z.string().min(1).max(200),
    content: z.string().max(2_000_000),
    visibility: z.enum(['members', 'accounting', 'managers']).default('members'),
    parentRevisionGuid: z.string().optional(),
    pageGuid: z.string().uuid().optional(),
    revisionGuid: z.string().uuid().optional(),
    attachments: z.array(attachment).max(20).default([]),
  })).mutation(async () => ({} as Record<string, unknown>)),
})
