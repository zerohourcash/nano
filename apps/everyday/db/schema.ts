import { sqliteTable, integer, text, real, index } from "drizzle-orm/sqlite-core";

export type RoleRights = {
  viewItems: boolean;
  createItems: boolean;
  editItems: boolean;
  deleteItems: boolean;
  transferItems: boolean;
  acceptTransfers: boolean;
  writeOff: boolean;
  replenish: boolean;
  inventory: boolean;
  viewHistory: boolean;
  viewReports: boolean;
  manageUsers: boolean;
  manageWorkspaces: boolean;
  manageStorages: boolean;
  manageSites: boolean;
  manageDictionaries: boolean;
};

export const DEFAULT_ROLE_RIGHTS: RoleRights = {
  viewItems: true,
  createItems: true,
  editItems: true,
  deleteItems: false,
  transferItems: true,
  acceptTransfers: true,
  writeOff: false,
  replenish: true,
  inventory: true,
  viewHistory: true,
  viewReports: true,
  manageUsers: false,
  manageWorkspaces: false,
  manageStorages: false,
  manageSites: false,
  manageDictionaries: false,
};

const ts = (name: string) =>
  integer(name, { mode: "timestamp_ms" }).notNull().$defaultFn(() => new Date());

const tsNull = (name: string) => integer(name, { mode: "timestamp_ms" });

export const workspaces = sqliteTable("workspaces", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  name: text("name").notNull(),
  timezone: text("timezone").notNull().default("Europe/Moscow"),
  internalIdPrefix: text("internal_id_prefix").notNull().default("ВН-"),
  comment: text("comment"),
  syncUrl: text("sync_url"),
  // ТЗ §8: группа может требовать фото-подтверждение при списании.
  requireWriteoffPhoto: integer("require_writeoff_photo", { mode: "boolean" })
    .notNull()
    .default(false),
  createdAt: ts("created_at"),
});

export const users = sqliteTable("users", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  fullName: text("full_name").notNull(),
  position: text("position"),
  phone: text("phone").notNull().unique(),
  avatarUrl: text("avatar_url"),
  status: text("status", { enum: ["active", "invited", "disabled"] }).notNull().default("active"),
  passwordHash: text("password_hash"),
  roleRights: text("role_rights", { mode: "json" }).$type<RoleRights>(),
  createdAt: ts("created_at"),
});

export const userWorkspaces = sqliteTable(
  "user_workspaces",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    userId: integer("user_id").notNull(),
    workspaceId: integer("workspace_id").notNull(),
  },
  (t) => [index("uw_user_idx").on(t.userId), index("uw_ws_idx").on(t.workspaceId)],
);

export const storages = sqliteTable("storages", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  name: text("name").notNull(),
  responsibleUserId: integer("responsible_user_id"),
  workspaceId: integer("workspace_id").notNull(),
  address: text("address"),
});

export const buildingSites = sqliteTable("building_sites", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  name: text("name").notNull(),
  responsibleUserId: integer("responsible_user_id"),
  workspaceId: integer("workspace_id").notNull(),
});

export const categories = sqliteTable("categories", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  name: text("name").notNull(),
  description: text("description"),
  workspaceId: integer("workspace_id").notNull(),
  type: text("type").notNull().default("category"),
});

export const brands = sqliteTable("brands", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  name: text("name").notNull(),
  description: text("description"),
  workspaceId: integer("workspace_id").notNull(),
  type: text("type").notNull().default("brand"),
});

export const statuses = sqliteTable("statuses", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  name: text("name").notNull(),
  description: text("description"),
  workspaceId: integer("workspace_id").notNull(),
  type: text("type").notNull().default("status"),
  slug: text("slug").notNull().default("in-stock"),
  color: text("color").notNull().default("#5E629B"),
  bg: text("bg").notNull().default("#EDEDF7"),
});

export const items = sqliteTable(
  "items",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    internalId: text("internal_id").notNull(),
    title: text("title").notNull(),
    categoryId: integer("category_id"),
    brandId: integer("brand_id"),
    statusId: integer("status_id"),
    responsibleUserId: integer("responsible_user_id"),
    buildingSiteId: integer("building_site_id"),
    storageId: integer("storage_id"),
    organizationNodeId: integer("organization_node_id"),
    workspaceId: integer("workspace_id").notNull(),
    serialNumber: text("serial_number"),
    cost: real("cost"),
    quantitative: integer("quantitative", { mode: "boolean" }).notNull().default(false),
    quantity: real("quantity"),
    unit: text("unit"),
    comment: text("comment"),
    qrCode: text("qr_code"),
    // Универсальный слой импорта: сохраняет идентификатор исходной системы и
    // расширенные поля (FaceKit, 1С и будущие интеграции) без потери данных.
    sourceSystem: text("source_system"),
    externalId: text("external_id"),
    metadata: text("metadata_json", { mode: "json" }).$type<Record<string, unknown>>(),
    notifyDate: tsNull("notify_date"),
    dueAt: text("due_at"),
    createdAt: ts("created_at"),
  },
  (t) => [
    index("items_ws_idx").on(t.workspaceId),
    index("items_resp_idx").on(t.responsibleUserId),
    index("items_storage_idx").on(t.storageId),
    index("items_site_idx").on(t.buildingSiteId),
    index("items_organization_node_idx").on(t.organizationNodeId),
    index("items_qr_idx").on(t.qrCode),
    index("items_internal_idx").on(t.internalId),
  ],
);

export const itemPhotos = sqliteTable("item_photos", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  itemId: integer("item_id").notNull(),
  url: text("url").notNull(),
  // ТЗ §5: уменьшенная копия для списков и контрольная сумма вложения.
  thumbUrl: text("thumb_url"),
  sha256: text("sha256"),
  isTitle: integer("is_title", { mode: "boolean" }).notNull().default(false),
});

export const itemDocuments = sqliteTable("item_documents", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  itemId: integer("item_id").notNull(),
  name: text("name").notNull(),
  url: text("url").notNull(),
});

export const itemComments = sqliteTable("item_comments", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  itemId: integer("item_id").notNull(),
  userId: integer("user_id").notNull(),
  text: text("text").notNull(),
  active: integer("active", { mode: "boolean" }).notNull().default(true),
  createdAt: ts("created_at"),
});

export const transfers = sqliteTable(
  "transfers",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    code: text("code"),
    itemId: integer("item_id").notNull(),
    fromUserId: integer("from_user_id").notNull(),
    toUserId: integer("to_user_id").notNull(),
    toStorageId: integer("to_storage_id"),
    buildingSiteId: integer("building_site_id"),
    workspaceId: integer("workspace_id").notNull(),
    quantity: real("quantity"),
    status: text("status", { enum: ["draft", "pending", "accepted", "rejected"] })
      .notNull()
      .default("pending"),
    photoUrl: text("photo_url"),
    comment: text("comment"),
    noConfirmation: integer("no_confirmation", { mode: "boolean" }).notNull().default(false),
    createdAt: ts("created_at"),
    completedAt: tsNull("completed_at"),
  },
  (t) => [
    index("tr_from_idx").on(t.fromUserId),
    index("tr_to_idx").on(t.toUserId),
    index("tr_item_idx").on(t.itemId),
  ],
);

export const historyEntries = sqliteTable(
  "history_entries",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    workspaceId: integer("workspace_id").notNull(),
    itemId: integer("item_id").notNull(),
    type: text("type", {
      enum: [
        "move",
        "transfer_send",
        "transfer_receive",
        "write_off",
        "replenish",
        "inventory",
        "create",
        "update",
      ],
    }).notNull(),
    actorUserId: integer("actor_user_id").notNull(),
    fromLabel: text("from_label"),
    toLabel: text("to_label"),
    quantityDelta: real("quantity_delta"),
    comment: text("comment"),
    // Колонка называется `hash` исторически; сейчас это просто уникальный
    // идентификатор операции для дедупликации при обмене с сервером.
    opId: text("hash").notNull(),
    createdAt: ts("created_at"),
  },
  (t) => [index("hist_ws_idx").on(t.workspaceId), index("hist_item_idx").on(t.itemId)],
);

export const inventorySessions = sqliteTable("inventory_sessions", {
  id: integer("id").primaryKey({ autoIncrement: true }),
  number: text("number").notNull(),
  workspaceId: integer("workspace_id").notNull(),
  status: text("status", { enum: ["in_progress", "completed"] }).notNull().default("in_progress"),
  startedBy: integer("started_by").notNull(),
  createdAt: ts("created_at"),
  completedAt: tsNull("completed_at"),
});

export const inventoryResults = sqliteTable(
  "inventory_results",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    sessionId: integer("session_id").notNull(),
    itemId: integer("item_id").notNull(),
    expectedQty: real("expected_qty"),
    actualQty: real("actual_qty"),
    checked: integer("checked", { mode: "boolean" }).notNull().default(false),
  },
  (t) => [index("inv_res_session_idx").on(t.sessionId)],
);

export const notifications = sqliteTable(
  "notifications",
  {
    id: integer("id").primaryKey({ autoIncrement: true }),
    userId: integer("user_id").notNull(),
    itemId: integer("item_id"),
    type: text("type").notNull(),
    title: text("title"),
    text: text("text").notNull(),
    read: integer("read", { mode: "boolean" }).notNull().default(false),
    createdAt: ts("created_at"),
  },
  (t) => [index("notif_user_idx").on(t.userId)],
);

export type Workspace = typeof workspaces.$inferSelect;
export type User = typeof users.$inferSelect;
export type UserWorkspace = typeof userWorkspaces.$inferSelect;
export type Storage = typeof storages.$inferSelect;
export type BuildingSite = typeof buildingSites.$inferSelect;
export type Category = typeof categories.$inferSelect;
export type Brand = typeof brands.$inferSelect;
export type Status = typeof statuses.$inferSelect;
export type Item = typeof items.$inferSelect;
export type ItemPhoto = typeof itemPhotos.$inferSelect;
export type ItemDocument = typeof itemDocuments.$inferSelect;
export type ItemComment = typeof itemComments.$inferSelect;
export type Transfer = typeof transfers.$inferSelect;
export type HistoryEntry = typeof historyEntries.$inferSelect;
export type InventorySession = typeof inventorySessions.$inferSelect;
export type InventoryResult = typeof inventoryResults.$inferSelect;
export type Notification = typeof notifications.$inferSelect;
