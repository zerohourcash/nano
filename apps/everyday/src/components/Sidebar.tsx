import { NavLink, Link, useNavigate } from 'react-router'
import {
  Wrench,
  Package,
  ArrowLeftRight,
  History,
  ClipboardCheck,
  BarChart3,
  Settings2,
  MessageCircle,
  QrCode,
  Bell,
  ChevronsLeft,
  ChevronsRight,
} from 'lucide-react'
import { cn } from '@/lib/utils'
import { useStore } from '@/lib/store'

const navItems = [
  { to: '/', label: 'Все ТМЦ', icon: Wrench, end: true },
  { to: '/invite', label: 'Пригласить по QR', icon: QrCode },
  { to: '/my', label: 'Мои ТМЦ', icon: Package },
  { to: '/transfers', label: 'Приём-передача', icon: ArrowLeftRight, badge: true },
  { to: '/history', label: 'История', icon: History },
  { to: '/notifications', label: 'Сроки и уведомления', icon: Bell },
  { to: '/chat', label: 'Чат группы', icon: MessageCircle },
  { to: '/inventory', label: 'Инвентаризация', icon: ClipboardCheck },
  { to: '/reports', label: 'Отчёты', icon: BarChart3 },
  { to: '/admin', label: 'Панель управления', icon: Settings2 },
]

export default function Sidebar() {
  const { sidebarCollapsed, toggleSidebar, transfersToSend, transfersToReceive, currentUser } = useStore()
  const navigate = useNavigate()
  const transferCount = transfersToSend + transfersToReceive

  return (
    <aside
      className={cn(
        'hidden lg:flex flex-col bg-surface border-r border-brand-100 sticky top-0 h-[100dvh] shrink-0 transition-[width] duration-200',
        sidebarCollapsed ? 'w-[72px]' : 'w-60'
      )}
    >
      {/* Логотип */}
      <Link
        to="/"
        className={cn(
          'flex items-center h-16 border-b border-brand-100/70 shrink-0',
          sidebarCollapsed ? 'justify-center px-0' : 'px-4'
        )}
      >
        {sidebarCollapsed ? (
          <img src="/logo-mark.svg" alt="Everyday" className="w-10 h-10" />
        ) : (
          <img src="/logo.svg" alt="Everyday" className="h-9 w-auto" />
        )}
      </Link>

      {/* Навигация */}
      <nav className="flex-1 overflow-y-auto py-3 px-2 space-y-1">
        {navItems.map((item) => (
          <NavLink
            key={item.to}
            to={item.to}
            end={item.end}
            title={sidebarCollapsed ? item.label : undefined}
            className={({ isActive }) =>
              cn(
                'relative group flex items-center gap-3 rounded-xl px-3 py-2.5 text-sm font-semibold transition-colors duration-150',
                sidebarCollapsed && 'justify-center px-0',
                isActive
                  ? 'bg-brand-50 text-brand-600'
                  : 'text-ink-500 hover:bg-brand-50/60 hover:text-ink-900'
              )
            }
          >
            {({ isActive }) => (
              <>
                {isActive && (
                  <span className="absolute left-0 top-1/2 -translate-y-1/2 w-[3px] h-6 rounded-r-full bg-accent" />
                )}
                <span className="relative shrink-0">
                  <item.icon size={20} strokeWidth={1.75} />
                  {item.badge && transferCount > 0 && sidebarCollapsed && (
                    <span className="absolute -top-1 -right-1 w-2.5 h-2.5 rounded-full bg-accent border-2 border-surface" />
                  )}
                </span>
                {!sidebarCollapsed && <span className="truncate">{item.label}</span>}
                {!sidebarCollapsed && item.badge && transferCount > 0 && (
                  <span className="ml-auto min-w-5 h-5 px-1.5 rounded-full bg-accent text-white text-[11px] leading-5 text-center font-mono font-semibold">
                    {transferCount}
                  </span>
                )}
                {sidebarCollapsed && (
                  <span className="pointer-events-none absolute left-full ml-3 top-1/2 -translate-y-1/2 whitespace-nowrap rounded-lg bg-ink-900 text-white text-xs font-semibold px-2.5 py-1.5 opacity-0 group-hover:opacity-100 transition-opacity z-50 shadow-hover">
                    {item.label}
                  </span>
                )}
              </>
            )}
          </NavLink>
        ))}
      </nav>

      {/* Карточка пользователя + сворачивание */}
      <div className="border-t border-brand-100/70 p-2 space-y-1">
        <button
          onClick={() => navigate('/profile')}
          title={sidebarCollapsed ? currentUser?.fullName : undefined}
          className={cn(
            'w-full flex items-center gap-3 rounded-xl px-2 py-2 hover:bg-brand-50 transition-colors',
            sidebarCollapsed && 'justify-center px-0'
          )}
        >
          <img
            src={currentUser?.avatarUrl || '/avatar-1.png'}
            alt={currentUser?.fullName ?? ''}
            className="w-9 h-9 rounded-full object-cover shrink-0 border border-brand-100"
          />
          {!sidebarCollapsed && (
            <span className="min-w-0 text-left">
              <span className="block text-sm font-semibold text-ink-900 truncate">{currentUser?.fullName ?? 'Гость'}</span>
              <span className="block text-xs text-ink-500 truncate">{currentUser?.position ?? ''}</span>
            </span>
          )}
        </button>
        <button
          onClick={toggleSidebar}
          className={cn(
            'w-full flex items-center gap-3 rounded-xl px-3 py-2 text-sm font-semibold text-ink-300 hover:text-brand-600 hover:bg-brand-50 transition-colors',
            sidebarCollapsed && 'justify-center px-0'
          )}
          title={sidebarCollapsed ? 'Развернуть' : 'Свернуть'}
        >
          {sidebarCollapsed ? <ChevronsRight size={18} /> : <ChevronsLeft size={18} />}
          {!sidebarCollapsed && 'Свернуть'}
        </button>
      </div>
    </aside>
  )
}
