import { relations } from "drizzle-orm";
import {
  workspaces,
  users,
  userWorkspaces,
  storages,
  buildingSites,
  categories,
  brands,
  statuses,
  items,
  itemPhotos,
  itemDocuments,
  itemComments,
  transfers,
  historyEntries,
  inventorySessions,
  inventoryResults,
  notifications,
} from "./schema";

export const workspacesRelations = relations(workspaces, ({ many }) => ({
  userWorkspaces: many(userWorkspaces),
  storages: many(storages),
  buildingSites: many(buildingSites),
  items: many(items),
}));

export const usersRelations = relations(users, ({ many }) => ({
  userWorkspaces: many(userWorkspaces),
  comments: many(itemComments),
  notifications: many(notifications),
  outgoingTransfers: many(transfers, { relationName: "transferFrom" }),
  incomingTransfers: many(transfers, { relationName: "transferTo" }),
}));

export const userWorkspacesRelations = relations(userWorkspaces, ({ one }) => ({
  user: one(users, { fields: [userWorkspaces.userId], references: [users.id] }),
  workspace: one(workspaces, { fields: [userWorkspaces.workspaceId], references: [workspaces.id] }),
}));

export const storagesRelations = relations(storages, ({ one }) => ({
  responsible: one(users, { fields: [storages.responsibleUserId], references: [users.id] }),
  workspace: one(workspaces, { fields: [storages.workspaceId], references: [workspaces.id] }),
}));

export const buildingSitesRelations = relations(buildingSites, ({ one }) => ({
  responsible: one(users, { fields: [buildingSites.responsibleUserId], references: [users.id] }),
  workspace: one(workspaces, { fields: [buildingSites.workspaceId], references: [workspaces.id] }),
}));

export const itemsRelations = relations(items, ({ one, many }) => ({
  workspace: one(workspaces, { fields: [items.workspaceId], references: [workspaces.id] }),
  category: one(categories, { fields: [items.categoryId], references: [categories.id] }),
  brand: one(brands, { fields: [items.brandId], references: [brands.id] }),
  status: one(statuses, { fields: [items.statusId], references: [statuses.id] }),
  responsible: one(users, { fields: [items.responsibleUserId], references: [users.id] }),
  buildingSite: one(buildingSites, { fields: [items.buildingSiteId], references: [buildingSites.id] }),
  storage: one(storages, { fields: [items.storageId], references: [storages.id] }),
  photos: many(itemPhotos),
  documents: many(itemDocuments),
  comments: many(itemComments),
}));

export const itemPhotosRelations = relations(itemPhotos, ({ one }) => ({
  item: one(items, { fields: [itemPhotos.itemId], references: [items.id] }),
}));

export const itemDocumentsRelations = relations(itemDocuments, ({ one }) => ({
  item: one(items, { fields: [itemDocuments.itemId], references: [items.id] }),
}));

export const itemCommentsRelations = relations(itemComments, ({ one }) => ({
  item: one(items, { fields: [itemComments.itemId], references: [items.id] }),
  user: one(users, { fields: [itemComments.userId], references: [users.id] }),
}));

export const transfersRelations = relations(transfers, ({ one }) => ({
  item: one(items, { fields: [transfers.itemId], references: [items.id] }),
  fromUser: one(users, { fields: [transfers.fromUserId], references: [users.id], relationName: "transferFrom" }),
  toUser: one(users, { fields: [transfers.toUserId], references: [users.id], relationName: "transferTo" }),
  toStorage: one(storages, { fields: [transfers.toStorageId], references: [storages.id] }),
  workspace: one(workspaces, { fields: [transfers.workspaceId], references: [workspaces.id] }),
}));

export const historyEntriesRelations = relations(historyEntries, ({ one }) => ({
  item: one(items, { fields: [historyEntries.itemId], references: [items.id] }),
  actor: one(users, { fields: [historyEntries.actorUserId], references: [users.id] }),
  workspace: one(workspaces, { fields: [historyEntries.workspaceId], references: [workspaces.id] }),
}));

export const inventorySessionsRelations = relations(inventorySessions, ({ one, many }) => ({
  workspace: one(workspaces, { fields: [inventorySessions.workspaceId], references: [workspaces.id] }),
  starter: one(users, { fields: [inventorySessions.startedBy], references: [users.id] }),
  results: many(inventoryResults),
}));

export const inventoryResultsRelations = relations(inventoryResults, ({ one }) => ({
  session: one(inventorySessions, { fields: [inventoryResults.sessionId], references: [inventorySessions.id] }),
  item: one(items, { fields: [inventoryResults.itemId], references: [items.id] }),
}));

export const notificationsRelations = relations(notifications, ({ one }) => ({
  user: one(users, { fields: [notifications.userId], references: [users.id] }),
  item: one(items, { fields: [notifications.itemId], references: [items.id] }),
}));
