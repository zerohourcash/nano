import { useEffect, useRef, useState } from 'react'
import { Link, useNavigate } from 'react-router'
import { Search, Bell, ChevronDown, Check, X, QrCode } from 'lucide-react'
import { cn } from '@/lib/utils'
import { useStore } from '@/lib/store'

export default function TopBar() {
  const {
    workspace,
    workspaces,
    setWorkspace,
    transfersToSend,
    transfersToReceive,
    unreadNotifications,
    currentUser,
  } = useStore()
  const navigate = useNavigate()
  const [wsOpen, setWsOpen] = useState(false)
  const [query, setQuery] = useState('')
  const wsRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const onClick = (e: MouseEvent) => {
      if (wsRef.current && !wsRef.current.contains(e.target as Node)) setWsOpen(false)
    }
    document.addEventListener('mousedown', onClick)
    return () => document.removeEventListener('mousedown', onClick)
  }, [])

  const submitSearch = () => {
    if (query.trim()) navigate(`/?q=${encodeURIComponent(query.trim())}`)
  }

  return (
    <header className="hidden lg:flex sticky top-0 z-40 h-[72px] items-center gap-5 border-b border-brand-100 bg-surface px-8">
      {/* Глобальный поиск */}
      <div className="relative w-[320px] shrink-0 xl:w-[360px]">
        <Search size={18} className="absolute left-3.5 top-1/2 -translate-y-1/2 text-ink-300" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && submitSearch()}
          placeholder="Поиск по названию или вн. номеру"
          className="h-11 w-full rounded-none border-0 border-b border-brand-100 bg-white pl-10 pr-16 text-sm text-ink-900 placeholder:text-ink-300 focus:border-brand-600"
        />
        {query ? (
          <button
            onClick={() => setQuery('')}
            className="absolute right-10 top-1/2 -translate-y-1/2 text-ink-300 hover:text-ink-500"
          >
            <X size={16} />
          </button>
        ) : null}
        <kbd className="absolute right-3 top-1/2 -translate-y-1/2 rounded-md border border-brand-100 bg-brand-50 px-1.5 py-0.5 text-[11px] font-mono font-semibold text-ink-500">
          ⌘K
        </kbd>
      </div>

      {/* Переключатель рабочего пространства */}
      <div ref={wsRef} className="relative">
        <button
          onClick={() => setWsOpen((v) => !v)}
          className="flex max-w-52 items-center gap-1.5 whitespace-nowrap rounded-xl px-3 py-2 text-sm text-ink-500 hover:bg-brand-50 transition-colors"
        >
          <span className="truncate font-semibold text-ink-900">{workspace?.name ?? '—'}</span>
          <ChevronDown size={16} className={cn('transition-transform', wsOpen && 'rotate-180')} />
        </button>
        {wsOpen && (
          <div className="absolute left-0 top-full mt-1 w-72 rounded-2xl border border-brand-100 bg-surface p-1.5 shadow-hover z-50">
            {workspaces.map((ws) => (
              <button
                key={ws.id}
                onClick={() => {
                  setWorkspace(ws)
                  setWsOpen(false)
                }}
                className={cn(
                  'flex w-full items-center gap-2.5 rounded-xl px-3 py-2.5 text-sm font-semibold transition-colors',
                  ws.id === workspace?.id ? 'bg-brand-50 text-brand-600' : 'text-ink-900 hover:bg-brand-50/60'
                )}
              >
                <img src="/logo-mark.svg" alt="" className="w-7 h-7 rounded-lg" />
                <span className="flex-1 text-left">{ws.name}</span>
                {ws.id === workspace?.id && <Check size={16} className="text-brand-600" />}
              </button>
            ))}
          </div>
        )}
      </div>

      <div className="flex-1" />

      <Link
        to="/invite"
        className="inline-flex items-center gap-1.5 rounded-xl bg-accent px-3.5 py-2 text-[13px] font-semibold text-white hover:bg-accent-hover"
      >
        <QrCode size={16} strokeWidth={2} />
        Пригласить
      </Link>

      {/* Плашка передач */}
      <Link
        to="/transfers"
        className="flex items-center gap-1.5 whitespace-nowrap rounded-full bg-warning-bg px-3.5 py-2 text-[13px] font-semibold text-warning hover:brightness-[0.98] transition"
      >
        Передачи <span className="font-mono">{transfersToSend + transfersToReceive}</span>
      </Link>

      {/* Индикатор синхронизации */}
      <div
        className="hidden 2xl:flex items-center gap-1.5 whitespace-nowrap text-[13px] font-semibold text-teal-dark"
        title="Журнал операций сохранён на сервере · закладка под офлайн-ноду"
      >
        <span className="w-2 h-2 rounded-full bg-teal" />
        Синхронизировано
      </div>

      {/* Уведомления */}
      <Link
        to="/notifications"
        className="relative flex items-center justify-center w-10 h-10 rounded-xl border border-brand-100 text-ink-500 hover:bg-brand-50 transition-colors"
      >
        <Bell size={18} strokeWidth={1.75} />
        {unreadNotifications > 0 && (
          <span className="absolute -top-1.5 -right-1.5 min-w-5 h-5 px-1 rounded-full bg-accent text-white text-[11px] leading-5 text-center font-mono font-semibold border-2 border-surface">
            {unreadNotifications}
          </span>
        )}
      </Link>

      {/* Аватар */}
      <Link to="/profile" className="shrink-0">
        <img
          src={currentUser?.avatarUrl || '/avatar-1.png'}
          alt={currentUser?.fullName ?? ''}
          className="w-10 h-10 rounded-full object-cover border border-brand-100 hover:shadow-card transition-shadow"
        />
      </Link>
    </header>
  )
}
