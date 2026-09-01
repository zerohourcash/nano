import { TRPCError } from '@trpc/server';
import { z } from 'zod';
import { createRouter, publicQuery } from './middleware';

export type BitTransaction = {
  guid: string;
  workspaceId: number;
  actorId: number;
  kind: 'mint' | 'transfer' | 'sale';
  memo: string | null;
  reference: string | null;
  senderAccountGuid: string | null;
  senderUserId: number | null;
  recipientUserId: number | null;
  amount: number;
  txHash: string;
  status: 'posted' | 'conflict' | 'pending';
  createdAt: string;
};

const workspaceInput = z.object({ workspaceId: z.number().int().positive().optional() }).optional();
const unavailable = () => {
  throw new TRPCError({
    code: 'PRECONDITION_FAILED',
    message: 'Bit работает через автономный Rust-узел',
  });
};

export const bitRouter = createRouter({
  balance: publicQuery
    .input(z.object({ workspaceId: z.number().int().positive().optional(), userId: z.number().int().positive().optional() }).optional())
    .query(async (): Promise<{ workspaceId: number; userId: number; currency: 'BIT'; minorUnit: 1; balance: number }> => unavailable()),
  recipients: publicQuery
    .input(workspaceInput)
    .query(async (): Promise<Array<{ id: number; guid: string; fullName: string; position: string | null; roleName: string | null }>> => unavailable()),
  myTransactions: publicQuery.input(workspaceInput).query(async (): Promise<BitTransaction[]> => unavailable()),
  transactions: publicQuery.input(workspaceInput).query(async (): Promise<BitTransaction[]> => unavailable()),
  transfer: publicQuery
    .input(z.object({ workspaceId: z.number().int().positive(), recipientUserId: z.number().int().positive(), amount: z.number().int().positive(), memo: z.string().max(500).optional(), reference: z.string().max(200).optional() }))
    .mutation(async (): Promise<BitTransaction> => unavailable()),
  mint: publicQuery
    .input(z.object({ workspaceId: z.number().int().positive(), recipientUserId: z.number().int().positive(), amount: z.number().int().positive(), memo: z.string().max(500).optional(), reference: z.string().max(200).optional() }))
    .mutation(async (): Promise<BitTransaction> => unavailable()),
  sale: publicQuery
    .input(z.object({ itemId: z.number().int().positive(), sellerUserId: z.number().int().positive(), amount: z.number().int().positive(), memo: z.string().max(500).optional() }))
    .mutation(async (): Promise<BitTransaction> => unavailable()),
});
