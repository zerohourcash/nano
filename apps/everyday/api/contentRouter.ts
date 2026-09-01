import { z } from 'zod'
import { createRouter, publicQuery } from './middleware'

const contentStatus = {
  mode: 'smart' as 'metadata' | 'smart' | 'full',
  blobs: 0,
  catalogEntries: 0,
  providers: 0,
  pinned: 0,
  pending: 0,
  bytes: 0,
  catalogBytes: 0,
  missing: 0,
  missingBytes: 0,
}

export const contentRouter = createRouter({
  status: publicQuery.query(async () => contentStatus),
  ingest: publicQuery
    .input(z.object({ workspaceId: z.number().int().positive(), dataUrl: z.string().min(1) }))
    .mutation(async () => ({ url: 'cas:' + '0'.repeat(64), hash: '0'.repeat(64), mime: 'application/octet-stream', size: 0 })),
  setMode: publicQuery
    .input(z.object({ mode: z.enum(['metadata', 'smart', 'full']) }))
    .mutation(async ({ input }) => ({ ...contentStatus, mode: input.mode })),
})
