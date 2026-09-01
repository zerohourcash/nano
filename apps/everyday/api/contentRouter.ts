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
  setMode: publicQuery
    .input(z.object({ mode: z.enum(['metadata', 'smart', 'full']) }))
    .mutation(async ({ input }) => ({ ...contentStatus, mode: input.mode })),
})
