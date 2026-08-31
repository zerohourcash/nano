import type { inferRouterOutputs } from '@trpc/server'
import type { AppRouter } from '../../../api/router'

export type RouterOutputs = inferRouterOutputs<AppRouter>

export type AdminUser = RouterOutputs['admin']['users']['list'][number]
export type WorkspaceDto = RouterOutputs['admin']['workspaces']['list'][number]
export type StorageDto = RouterOutputs['admin']['storages']['list'][number]
export type SiteDto = RouterOutputs['admin']['buildingSites']['list'][number]
export type DictEntryDto = RouterOutputs['admin']['dictionaries']['list'][number]
export type ItemDto = RouterOutputs['reports']['allItems'][number]
export type DictKind = 'categories' | 'brands' | 'statuses'
