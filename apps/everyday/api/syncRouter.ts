import { z } from "zod";
import { createRouter, publicQuery } from "./middleware";

export const syncRouter = createRouter({
  status: publicQuery.query(async () => ({
    nodeId: "",
    name: "",
    role: "" as "node" | "server" | "mesh",
    upstream: null as string | null,
    lastSync: null as string | null,
    lastError: null as string | null,
    url: "",
    localUrl: "",
    peers: [] as Array<{
      id: number;
      nodeId: string | null;
      url: string;
      name: string | null;
      lastSeen: string | null;
      lastSync: string | null;
      lastError: string | null;
    }>,
    openConflicts: 0,
    bytesSent: 0,
    bytesReceived: 0,
    syncSuccesses: 0,
  })),
  audit: publicQuery.query(async () => ({
    healthy: true,
    checkedAt: "",
    database: "ok",
    ledgerVerified: 0,
    chatVerified: 0,
    ledgerError: null as string | null,
    chatError: null as string | null,
    snapshotError: null as string | null,
    snapshotHash: null as string | null,
    lastEventAt: null as string | null,
    orphanHistory: 0,
    missingGuids: 0,
    missingBlobs: 0,
    pendingDownloads: 0,
    counts: { workspaces: 0, users: 0, items: 0, history: 0, messages: 0, organizationNodes: 0, blobs: 0 },
    ledgerHeads: [] as Array<{ workspaceGuid: string; publicKey: string; head: string; createdAt: string }>,
  })),
  peers: publicQuery.query(async () => [] as Array<{
    id: number;
    nodeId: string | null;
    url: string;
    name: string | null;
    lastSeen: string | null;
    lastSync: string | null;
    lastError: string | null;
  }>),
  addPeer: publicQuery
    .input(z.object({ url: z.string().min(4), name: z.string().optional() }))
    .mutation(async ({ input }) => ({ ok: true, url: input.url })),
  removePeer: publicQuery
    .input(z.object({ url: z.string().min(4) }))
    .mutation(async () => ({ ok: true, removed: 1 })),
  pullNow: publicQuery
    .input(z.object({ url: z.string().min(4).optional() }).optional())
    .mutation(async () => ({ ok: true, queued: true })),
  conflicts: publicQuery.query(async () => [] as Array<{
    id: number;
    workspaceId: number | null;
    itemId: number | null;
    itemGuid: string | null;
    status: string;
    description: string;
    leftLabel: string | null;
    rightLabel: string | null;
    createdAt: string;
    item: { id: number; title: string; internalId: string } | null;
  }>),
  resolveConflict: publicQuery
    .input(z.object({ id: z.number().int().positive(), responsibleUserId: z.number().int().positive().nullable().optional() }))
    .mutation(async () => ({ ok: true })),
});

export const backupRouter = createRouter({
  export: publicQuery
    .input(z.object({ password: z.string().min(12).max(128) }))
    .mutation(async () => ({
      v: 1,
      alg: "chacha20poly1305",
      nonce: "",
      ciphertext: "",
      sha256: "",
    })),
  import: publicQuery
    .input(z.object({ password: z.string().min(12).max(128), blob: z.any() }))
    .mutation(async () => ({ ok: true, workspaces: 0, users: 0, items: 0, ops: 0, skipped: 0, conflicts: 0 })),
});
