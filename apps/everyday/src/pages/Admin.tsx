import { useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { Building2, Layers, ListTree, Radio, ShieldAlert, Users, Warehouse } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import { cn } from '@/lib/utils'
import { ToastProvider } from './admin/ui'
import UsersSection from './admin/UsersSection'
import WorkspacesSection from './admin/WorkspacesSection'
import StoragesSection from './admin/StoragesSection'
import SitesSection from './admin/SitesSection'
import DictionariesSection from './admin/DictionariesSection'
import OfflineNodesSection from './admin/OfflineNodesSection'
import RequestsSection from './admin/RequestsSection'

type SectionId = 'users' | 'workspaces' | 'storages' | 'sites' | 'dictionaries' | 'requests' | 'offline'

const SECTIONS: { id: SectionId; label: string; icon: LucideIcon; soon?: boolean }[] = [
  { id: 'users', label: 'Пользователи', icon: Users },
  { id: 'workspaces', label: 'Пространства', icon: Layers },
  { id: 'storages', label: 'Склады', icon: Warehouse },
  { id: 'sites', label: 'Объекты', icon: Building2 },
  { id: 'dictionaries', label: 'Справочники', icon: ListTree },
  { id: 'requests', label: 'Заявки и неисправности', icon: ShieldAlert },
  { id: 'offline', label: 'Офлайн-узлы', icon: Radio },
]

function SectionNav({
  active,
  onSelect,
  vertical,
}: {
  active: SectionId
  onSelect: (id: SectionId) => void
  vertical: boolean
}) {
  return (
    <nav
      className={cn(
        vertical
          ? 'hidden w-56 shrink-0 flex-col gap-1 lg:flex'
          : 'flex gap-1 overflow-x-auto pb-1 lg:hidden'
      )}
      aria-label="Разделы панели управления"
    >
      {SECTIONS.map((s) => {
        const isActive = s.id === active
        return (
          <button
            key={s.id}
            type="button"
            onClick={() => onSelect(s.id)}
            className={cn(
              'relative flex shrink-0 items-center gap-2.5 rounded-xl px-3.5 py-2.5 text-sm font-semibold transition',
              isActive
                ? 'bg-brand-50 text-brand-600'
                : 'text-ink-500 hover:bg-brand-50/60 hover:text-ink-900'
            )}
          >
            {isActive && (
              <motion.span
                layoutId={vertical ? 'admin-nav-bar' : 'admin-nav-bar-m'}
                className={cn(
                  'absolute rounded-full bg-accent',
                  vertical ? 'left-0 top-2 bottom-2 w-[3px]' : 'left-2 right-2 bottom-0 h-[3px]'
                )}
                transition={{ duration: 0.2 }}
              />
            )}
            <s.icon size={vertical ? 18 : 16} className="shrink-0" />
            <span className="whitespace-nowrap">{s.label}</span>
            {s.soon && (
              <span className="rounded-full bg-brand-100/70 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-brand-700">
                скоро
              </span>
            )}
          </button>
        )
      })}
    </nav>
  )
}

export default function Admin() {
  const [section, setSection] = useState<SectionId>('users')

  return (
    <ToastProvider>
      <div className="space-y-4 lg:space-y-6">
        <h1 className="text-2xl lg:text-[28px] leading-9 font-bold tracking-[-0.01em] text-ink-900">
          Панель управления
        </h1>

        <div className="flex flex-col items-start gap-4 lg:flex-row lg:gap-6">
          <SectionNav vertical active={section} onSelect={setSection} />
          <div className="w-full min-w-0 flex-1">
            <SectionNav vertical={false} active={section} onSelect={setSection} />
            <AnimatePresence mode="wait">
              <motion.div
                key={section}
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0 }}
                transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
              >
                {section === 'users' && <UsersSection />}
                {section === 'workspaces' && <WorkspacesSection />}
                {section === 'storages' && <StoragesSection />}
                {section === 'sites' && <SitesSection />}
                {section === 'dictionaries' && <DictionariesSection />}
                {section === 'requests' && <RequestsSection />}
                {section === 'offline' && <OfflineNodesSection />}
              </motion.div>
            </AnimatePresence>
          </div>
        </div>
      </div>
    </ToastProvider>
  )
}
