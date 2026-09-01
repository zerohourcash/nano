import { z } from 'zod'
import { createRouter, publicQuery } from './middleware'

type Identity = { workspaceId: number; publicKey: string; signingKey: string; destination: string; createdAt?: string }
type Contact = { guid: string; name: string; remoteWorkspaceGuid: string; destination: string; encryptionKey: string; signingKey: string; active: boolean; createdAt?: string }

export const interorgRouter = createRouter({
  identity: publicQuery.input(z.object({ workspaceId: z.number().int().positive() })).query(async () => null as Identity | null),
  ensureIdentity: publicQuery.input(z.object({ workspaceId: z.number().int().positive() })).mutation(async ({ input }) => ({ ...input, publicKey: '', signingKey: '', destination: '' } as Identity)),
  contacts: publicQuery.input(z.object({ workspaceId: z.number().int().positive() })).query(async () => [] as Contact[]),
  trustContact: publicQuery.input(z.object({
    workspaceId: z.number().int().positive(),
    name: z.string().min(1).max(120),
    remoteWorkspaceGuid: z.string().min(1).max(128),
    encryptionKey: z.string().min(40).max(64),
    signingKey: z.string().min(40).max(64),
  })).mutation(async ({ input }) => ({ guid: crypto.randomUUID(), destination: '', active: true, ...input } as Contact)),
  inbox: publicQuery.input(z.object({ workspaceId: z.number().int().positive() })).query(async () => [] as Array<{
    envelopeId: string
    transactionId: string
    kind: string
    body: unknown
    receivedAt: string
    accepted: boolean
    contact: { guid: string; name: string; remoteWorkspaceGuid: string }
  }>),
  send: publicQuery.input(z.object({
    workspaceId: z.number().int().positive(),
    contactGuid: z.string().uuid(),
    transactionId: z.string().uuid(),
    kind: z.string().min(1).max(80),
    body: z.unknown(),
  })).mutation(async ({ input }) => ({ ok: true, queued: true, envelopeId: '', destination: '', ledgerHash: '', transactionId: input.transactionId })),
  accept: publicQuery.input(z.object({ workspaceId: z.number().int().positive(), envelopeId: z.string().uuid() })).mutation(async ({ input }) => ({ ok: true, envelopeId: input.envelopeId, transactionId: '', ledgerHash: '' })),
})
