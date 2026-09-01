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
    workspaceScopeMode: "all" as "disabled" | "all" | "restricted" | "capabilities",
    workspaceScope: [] as string[],
    capabilityCount: 1,
  })),
  audit: publicQuery.query(async () => ({
    healthy: true,
    checkedAt: "",
    database: "ok",
    ledgerVerified: 0,
    chatVerified: 0,
    accountingVerified: true,
    knowledgeVerified: true,
    accountingError: null as string | null,
    knowledgeError: null as string | null,
    ledgerError: null as string | null,
    chatError: null as string | null,
    deviceError: null as string | null,
    deviceRegistryVerified: true,
    deviceProofsVerified: 0,
    custodyError: null as string | null,
    custodyVerified: true,
    custodyEntriesVerified: 0,
    itemTombstoneError: null as string | null,
    itemTombstonesVerified: 0,
    itemCommentError: null as string | null,
    itemCommentsVerified: 0,
    faultError: null as string | null,
    faultRecordsVerified: 0,
    changeRequestError: null as string | null,
    changeRequestRecordsVerified: 0,
    configError: null as string | null,
    configVersionsVerified: 0,
    membershipError: null as string | null,
    membershipVerified: true,
    snapshotError: null as string | null,
    snapshotHash: null as string | null,
    lastEventAt: null as string | null,
    orphanHistory: 0,
    missingGuids: 0,
    missingBlobs: 0,
    missingReferencedBlobs: 0,
    pendingDownloads: 0,
    counts: { workspaces: 0, users: 0, devices: 0, custodyEntries: 0, itemTombstones: 0, itemComments: 0, faultRecords: 0, changeRequestRecords: 0, configVersions: 0, items: 0, history: 0, messages: 0, organizationNodes: 0, blobs: 0, accountingTransactions: 0, accountingLines: 0, knowledgePages: 0, knowledgeRevisions: 0, membershipVersions: 0 },
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
  nodeKeys: publicQuery.query(async () => ({
    strict: false,
    trusted: [] as Array<{ publicKey: string; label: string | null; approvedBy: number | null; source: string; createdAt: string }>,
    pending: [] as Array<{ publicKey: string; peerUrl: string | null; nodeName: string | null; firstSeen: string; lastSeen: string }>,
  })),
  diagnostics: publicQuery.query(async () => ({
    unresolved: 0,
    events: [] as Array<{
      id: number;
      severity: "info" | "warning" | "error" | "critical";
      component: string;
      code: string;
      message: string;
      context: { peer?: string } | null;
      firstAt: string;
      lastAt: string;
      count: number;
      resolvedAt: string | null;
    }>,
  })),
  clearDiagnostics: publicQuery.input(z.object({}).optional()).mutation(async () => ({ ok: true, removed: 0 })),
  reportTransportStatus: publicQuery.input(z.object({
    transport: z.literal('ble'),
    message: z.string().max(500),
    error: z.boolean(),
  })).mutation(async ({ input }) => ({ ok: true, active: input.error })),
  exportBundle: publicQuery.input(z.object({ workspaceGuid: z.string().max(128).optional() }).optional()).query(async () => ({
    format: "everyday-sync-bundle" as const,
    version: 2 as const,
    createdAt: "",
    cipher: "XChaCha20-Poly1305" as const,
    kdf: "HKDF-SHA256" as const,
    nonce: "",
    ciphertext: "",
  })),
  importBundle: publicQuery
    .input(z.object({ bundle: z.unknown() }))
    .mutation(async () => ({ ok: true, imported: 0, conflicts: 0 })),
  approveNodeKey: publicQuery.input(z.object({ publicKey: z.string().min(32), label: z.string().max(100).optional() })).mutation(async () => ({ strict: true, trusted: [], pending: [] })),
  revokeNodeKey: publicQuery.input(z.object({ publicKey: z.string().min(32) })).mutation(async () => ({ strict: true, trusted: [], pending: [] })),
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
