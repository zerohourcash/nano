import { createContext, useContext, useMemo, useState } from 'react'
import type { ReactNode } from 'react'
import { trpc } from '@/providers/trpc'

interface Workspace {
  id: number
  name: string
  internalIdPrefix?: string
}

interface User {
  id: number
  fullName: string
  position: string | null
  phone: string
  avatarUrl: string | null
}

interface AppStore {
  workspace: Workspace | null
  workspaces: Workspace[]
  setWorkspace: (ws: Workspace) => void
  currentUser: User | null
  selectedToolIds: Set<string>
  toggleToolSelected: (id: string) => void
  setToolSelected: (id: string, selected: boolean) => void
  clearSelection: () => void
  selectionMode: boolean
  setSelectionMode: (v: boolean) => void
  transfersToSend: number
  transfersToReceive: number
  unreadNotifications: number
  sidebarCollapsed: boolean
  toggleSidebar: () => void
}

const StoreContext = createContext<AppStore | null>(null)

export function StoreProvider({ children }: { children: ReactNode }) {
  const meQ = trpc.auth.me.useQuery(undefined, { retry: 0 })
  const wsQ = trpc.meta.workspaces.useQuery(undefined, { enabled: !!meQ.data, retry: 0 })
  const countsQ = trpc.meta.transferCounts.useQuery(undefined, { enabled: !!meQ.data, retry: 0 })
  const unreadQ = trpc.notifications.unreadCount.useQuery(undefined, { enabled: !!meQ.data, retry: 0 })

  const [workspaceOverride, setWorkspace] = useState<Workspace | null>(null)
  const [selectedToolIds, setSelectedToolIds] = useState<Set<string>>(new Set())
  const [selectionMode, setSelectionMode] = useState(false)
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)

  const workspaces = useMemo(() => wsQ.data ?? [], [wsQ.data])
  const workspace = workspaceOverride ?? workspaces[0] ?? null

  const value = useMemo<AppStore>(() => {
    const toggleToolSelected = (id: string) => {
      setSelectedToolIds((prev) => {
        const next = new Set(prev)
        if (next.has(id)) next.delete(id)
        else next.add(id)
        if (next.size === 0) setSelectionMode(false)
        return next
      })
    }
    const setToolSelected = (id: string, selected: boolean) => {
      setSelectedToolIds((prev) => {
        const next = new Set(prev)
        if (selected) next.add(id)
        else next.delete(id)
        if (next.size === 0) setSelectionMode(false)
        return next
      })
    }
    const clearSelection = () => {
      setSelectedToolIds(new Set())
      setSelectionMode(false)
    }

    return {
      workspace,
      workspaces,
      setWorkspace,
      currentUser: meQ.data
        ? {
            id: meQ.data.id,
            fullName: meQ.data.fullName,
            position: meQ.data.position,
            phone: meQ.data.phone,
            avatarUrl: meQ.data.avatarUrl,
          }
        : null,
      selectedToolIds,
      toggleToolSelected,
      setToolSelected,
      clearSelection,
      selectionMode,
      setSelectionMode,
      transfersToSend: countsQ.data?.outgoing ?? 0,
      transfersToReceive: countsQ.data?.incoming ?? 0,
      unreadNotifications: unreadQ.data?.count ?? 0,
      sidebarCollapsed,
      toggleSidebar: () => setSidebarCollapsed((v) => !v),
    }
  }, [
    workspace,
    workspaces,
    meQ.data,
    selectedToolIds,
    selectionMode,
    sidebarCollapsed,
    countsQ.data,
    unreadQ.data,
  ])

  return <StoreContext.Provider value={value}>{children}</StoreContext.Provider>
}

export function useStore(): AppStore {
  const ctx = useContext(StoreContext)
  if (!ctx) throw new Error('useStore must be used within <StoreProvider>')
  return ctx
}
