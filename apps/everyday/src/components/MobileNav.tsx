import { useState } from 'react'
import { NavLink, Link, useNavigate } from 'react-router'
import {
  Wrench,
  Package,
  ArrowLeftRight,
  Menu,
  QrCode,
  Bell,
  Search,
  ChevronDown,
  History,
  ClipboardCheck,
  BarChart3,
  Settings2,
  MessageCircle,
  BookOpen,
  UserRound,
  X,
  Coins,
  Mic,
} from 'lucide-react'
import { AnimatePresence, motion } from 'framer-motion'
import { cn } from '@/lib/utils'
import { useStore } from '@/lib/store'

const moreItems = [
  { to: '/voice', label: 'Голосовые команды', icon: Mic },
  { to: '/invite', label: 'Пригласить по QR', icon: QrCode },
  { to: '/notifications', label: 'Сроки и уведомления', icon: Bell },
  { to: '/history', label: 'История', icon: History },
  { to: '/chat', label: 'Чат группы', icon: MessageCircle },
  { to: '/knowledge', label: 'База знаний', icon: BookOpen },
  { to: '/bit', label: 'Кошелёк Bit', icon: Coins },
  { to: '/inventory', label: 'Инвентаризация', icon: ClipboardCheck },
  { to: '/reports', label: 'Отчёты', icon: BarChart3 },
  { to: '/admin', label: 'Панель управления', icon: Settings2 },
  { to: '/profile', label: 'Мой профиль', icon: UserRound },
]

/** Мобильный каркас: верхняя панель 56px + нижняя навигация с FAB (design.md §5) */
export default function MobileNav() {
  const { workspace, workspaces, setWorkspace, currentUser, transfersToSend, transfersToReceive, unreadNotifications } = useStore()
  const [sheetOpen, setSheetOpen] = useState(false)
  const [wsOpen, setWsOpen] = useState(false)
  const navigate = useNavigate()
  const transferCount = transfersToSend + transfersToReceive

  const tabClass = ({ isActive }: { isActive: boolean }) =>
    cn(
      'relative flex flex-col items-center justify-center gap-0.5 flex-1 h-full text-[11px] font-semibold transition-colors',
      isActive ? 'text-brand-600' : 'text-ink-300'
    )

  return (
    <>
      {/* Верхняя панель (мобайл) */}
      <header className="lg:hidden sticky top-0 z-40 h-14 flex items-center gap-2 bg-surface px-3 shadow-card">
        <Link to="/" className="shrink-0">
          <img src="/logo-mark.svg" alt="Everyday" className="w-9 h-9" />
        </Link>
        <div className="relative">
          <button
            onClick={() => setWsOpen((v) => !v)}
            className="flex items-center gap-1 rounded-lg px-2 py-1.5 text-[13px] font-semibold text-ink-900 hover:bg-brand-50 max-w-[160px]"
          >
            <span className="truncate">{workspace?.name ?? 'Everyday'}</span>
            <ChevronDown size={14} className="shrink-0 text-ink-300" />
          </button>
          {wsOpen && (
            <div className="absolute left-0 top-full mt-1 w-64 rounded-2xl border border-brand-100 bg-surface p-1.5 shadow-hover z-50">
              {workspaces.map((ws) => (
                <button
                  key={ws.id}
                  onClick={() => {
                    setWorkspace(ws)
                    setWsOpen(false)
                  }}
                  className={cn(
                    'flex w-full items-center rounded-xl px-3 py-2.5 text-sm font-semibold',
                    ws.id === workspace?.id ? 'bg-brand-50 text-brand-600' : 'text-ink-900 hover:bg-brand-50/60'
                  )}
                >
                  {ws.name}
                </button>
              ))}
            </div>
          )}
        </div>
        <div className="flex-1" />
        <Link
          to="/?focus=search"
          className="flex items-center justify-center w-10 h-10 rounded-xl text-ink-500 hover:bg-brand-50"
        >
          <Search size={20} strokeWidth={1.75} />
        </Link>
        <Link
          to="/notifications"
          className="relative flex items-center justify-center w-10 h-10 rounded-xl text-ink-500 hover:bg-brand-50"
        >
          <Bell size={20} strokeWidth={1.75} />
          {unreadNotifications > 0 && (
            <span className="absolute top-1 right-1 min-w-4 h-4 px-0.5 rounded-full bg-accent text-white text-[10px] leading-4 text-center font-mono font-semibold">
              {unreadNotifications}
            </span>
          )}
        </Link>
      </header>

      {/* Нижняя навигация + FAB */}
      <nav className="lg:hidden fixed bottom-0 inset-x-0 z-40 h-16 bg-surface shadow-[0_-4px_16px_rgba(48,52,102,.10)] flex items-stretch px-2">
        <NavLink to="/" end className={tabClass}>
          <Wrench size={22} strokeWidth={1.75} />
          Каталог
        </NavLink>
        <NavLink to="/my" className={tabClass}>
          <Package size={22} strokeWidth={1.75} />
          Мои
        </NavLink>

        {/* FAB */}
        <div className="flex-1 flex items-start justify-center">
          <button
            onClick={() => navigate('/scan')}
            aria-label="Сканировать QR"
            className="-translate-y-3 w-14 h-14 rounded-full bg-accent text-white flex items-center justify-center shadow-hover active:scale-95 transition-transform"
          >
            <QrCode size={26} strokeWidth={2.25} />
          </button>
        </div>

        <NavLink to="/transfers" className={tabClass}>
          <span className="relative">
            <ArrowLeftRight size={22} strokeWidth={1.75} />
            {transferCount > 0 && (
              <span className="absolute -top-1.5 -right-2 min-w-4 h-4 px-0.5 rounded-full bg-accent text-white text-[10px] leading-4 text-center font-mono font-semibold">
                {transferCount}
              </span>
            )}
          </span>
          Передачи
        </NavLink>
        <button
          onClick={() => setSheetOpen(true)}
          className="flex flex-col items-center justify-center gap-0.5 flex-1 h-full text-[11px] font-semibold text-ink-300"
        >
          <Menu size={22} strokeWidth={1.75} />
          Ещё
        </button>
      </nav>

      {/* Bottom sheet «Ещё» */}
      <AnimatePresence>
        {sheetOpen && (
          <>
            <motion.div
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.16 }}
              onClick={() => setSheetOpen(false)}
              className="lg:hidden fixed inset-0 z-50 bg-[rgba(48,52,102,.45)] backdrop-blur-sm"
            />
            <motion.div
              initial={{ y: '100%' }}
              animate={{ y: 0 }}
              exit={{ y: '100%' }}
              transition={{ type: 'spring', stiffness: 300, damping: 30 }}
              className="lg:hidden fixed bottom-0 inset-x-0 z-50 bg-surface rounded-t-modal p-4 pb-8 shadow-modal"
            >
              <div className="flex items-center justify-between mb-3">
                <div className="mx-auto w-10 h-1 rounded-full bg-brand-100 absolute left-1/2 -translate-x-1/2 top-2" />
                <h3 className="text-[17px] font-semibold text-ink-900 mt-2">Все разделы</h3>
                <button
                  onClick={() => setSheetOpen(false)}
                  className="mt-2 w-8 h-8 flex items-center justify-center rounded-lg text-ink-300 hover:bg-brand-50"
                >
                  <X size={18} />
                </button>
              </div>
              <div className="grid grid-cols-2 gap-2">
              {moreItems.filter((item) => item.to !== '/bit' || currentUser?.roleRights.useBit === true).map((item) => (
                  <button
                    key={item.to}
                    onClick={() => {
                      setSheetOpen(false)
                      navigate(item.to)
                    }}
                    className="flex items-center gap-3 rounded-xl border border-brand-100 px-3 py-3 text-sm font-semibold text-ink-900 hover:bg-brand-50 transition-colors"
                  >
                    <item.icon size={20} strokeWidth={1.75} className="text-brand-600" />
                    {item.label}
                  </button>
                ))}
              </div>
            </motion.div>
          </>
        )}
      </AnimatePresence>
    </>
  )
}
