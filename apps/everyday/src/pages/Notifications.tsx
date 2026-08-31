import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router'
import { AnimatePresence, motion } from 'framer-motion'
import {
  AlarmClock,
  AlertTriangle,
  ArrowLeftRight,
  Check,
  CheckCircle2,
  CheckCheck,
  ClipboardCheck,
  Info,
} from 'lucide-react'
import type { LucideIcon } from 'lucide-react'
import { format, isToday, isYesterday } from 'date-fns'
import { ru } from 'date-fns/locale'
import type { inferRouterOutputs } from '@trpc/server'
import type { AppRouter } from '../../api/router'
import { trpc } from '@/providers/trpc'
import { cn } from '@/lib/utils'

type RouterOutputs = inferRouterOutputs<AppRouter>
type Notification = RouterOutputs['notifications']['list'][number]

// ─── Вспомогательное ─────────────────────────────────────────────────────────

type FilterKey = 'all' | 'transfer' | 'reminder' | 'inventory' | 'system'

const FILTERS: { key: FilterKey; label: string }[] = [
  { key: 'all', label: 'Все' },
  { key: 'transfer', label: 'Передачи' },
  { key: 'reminder', label: 'Напоминания' },
  { key: 'inventory', label: 'Инвентаризация' },
  { key: 'system', label: 'Системные' },
]

interface TypeMeta {
  Icon: LucideIcon
  bg: string
  color: string
}

/** Иконка и цвет подложки по типу уведомления (design §notifications.3) */
function typeMeta(n: Notification): TypeMeta {
  const text = `${n.title ?? ''} ${n.text}`
  if (/просроч/i.test(text)) return { Icon: AlertTriangle, bg: '#FAD8D1', color: '#D64545' }
  switch (n.type) {
    case 'transfer':
      return { Icon: ArrowLeftRight, bg: '#D8F2F0', color: '#2E8E86' }
    case 'reminder':
      return { Icon: AlarmClock, bg: '#FBFCC8', color: '#A87C0F' }
    case 'inventory':
      return { Icon: ClipboardCheck, bg: '#EDEDF7', color: '#5E629B' }
    default:
      return { Icon: Info, bg: '#EDEDF7', color: '#5E629B' }
  }
}

function dayLabel(d: Date): string {
  if (isToday(d)) return 'Сегодня'
  if (isYesterday(d)) return 'Вчера'
  const sameYear = d.getFullYear() === new Date().getFullYear()
  return format(d, sameYear ? 'd MMMM' : 'd MMMM yyyy', { locale: ru })
}

interface DayGroup {
  label: string
  items: Notification[]
}

function groupByDay(list: Notification[]): DayGroup[] {
  const groups: DayGroup[] = []
  for (const n of list) {
    const label = dayLabel(new Date(n.createdAt))
    const last = groups[groups.length - 1]
    if (last && last.label === label) last.items.push(n)
    else groups.push({ label, items: [n] })
  }
  return groups
}

// ─── Страница ────────────────────────────────────────────────────────────────

export default function Notifications() {
  const navigate = useNavigate()
  const utils = trpc.useUtils()
  const [filter, setFilter] = useState<FilterKey>('all')
  const [toast, setToast] = useState<string | null>(null)

  const listQ = trpc.notifications.list.useQuery()
  const unreadQ = trpc.notifications.unreadCount.useQuery()

  const markRead = trpc.notifications.markRead.useMutation({
    onSuccess: () => {
      utils.notifications.list.invalidate()
      utils.notifications.unreadCount.invalidate()
    },
  })
  const markAllRead = trpc.notifications.markAllRead.useMutation({
    onSuccess: () => {
      utils.notifications.list.invalidate()
      utils.notifications.unreadCount.invalidate()
      setToast('Все уведомления прочитаны')
    },
  })

  useEffect(() => {
    if (!toast) return
    const t = setTimeout(() => setToast(null), 4000)
    return () => clearTimeout(t)
  }, [toast])

  const all = useMemo(() => listQ.data ?? [], [listQ.data])
  const filtered = useMemo(
    () => (filter === 'all' ? all : all.filter((n) => n.type === filter)),
    [all, filter]
  )
  const groups = useMemo(() => groupByDay(filtered), [filtered])
  const unreadCount = unreadQ.data?.count ?? all.filter((n) => !n.read).length

  const countFor = (key: FilterKey) =>
    key === 'all' ? all.length : all.filter((n) => n.type === key).length

  const openNotification = (n: Notification) => {
    if (!n.read) markRead.mutate({ id: n.id })
    if (n.itemId) navigate(`/tool/${n.itemId}`)
  }

  return (
    <div className="space-y-4 lg:space-y-6">
      {/* ── Заголовок ── */}
      <motion.div
        initial={{ opacity: 0, y: 16 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.28, ease: [0.22, 1, 0.36, 1] }}
        className="flex flex-wrap items-center gap-3"
      >
        <h1 className="text-2xl lg:text-[28px] leading-9 font-bold tracking-[-0.01em] text-ink-900">
          Уведомления
        </h1>
        {unreadCount > 0 && (
          <motion.span
            key={unreadCount}
            initial={{ scale: 1.25 }}
            animate={{ scale: 1 }}
            transition={{ duration: 0.3 }}
            className="inline-flex items-center rounded-full bg-accent px-2.5 py-1 text-caption text-white"
          >
            {unreadCount} новых
          </motion.span>
        )}
        <div className="ml-auto">
          <button
            type="button"
            onClick={() => markAllRead.mutate()}
            disabled={unreadCount === 0 || markAllRead.isPending}
            className="inline-flex h-10 items-center gap-2 rounded-xl px-4 text-sm font-semibold text-brand-600 transition-colors hover:bg-brand-50 disabled:opacity-40 disabled:hover:bg-transparent"
          >
            {markAllRead.isPending ? (
              <Check size={16} className="animate-pulse" />
            ) : (
              <CheckCheck size={16} />
            )}
            Отметить все прочитанными
          </button>
        </div>
      </motion.div>

      {/* ── Фильтр-чипы ── */}
      <motion.div
        initial={{ opacity: 0, y: 16 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.28, delay: 0.05, ease: [0.22, 1, 0.36, 1] }}
        className="flex flex-wrap gap-2"
      >
        {FILTERS.map((f) => {
          const active = filter === f.key
          return (
            <button
              key={f.key}
              type="button"
              onClick={() => setFilter(f.key)}
              className={cn(
                'relative h-9 rounded-full px-4 text-sm font-semibold transition-colors',
                active
                  ? 'text-white'
                  : 'bg-surface border border-brand-100 text-ink-900 hover:bg-brand-50'
              )}
            >
              {active && (
                <motion.span
                  layoutId="notif-chip"
                  className="absolute inset-0 rounded-full bg-brand-600"
                  transition={{ duration: 0.2, ease: [0.22, 1, 0.36, 1] }}
                />
              )}
              <span className="relative z-10">
                {f.label}{' '}
                <span className={cn('font-mono-num', active ? 'text-white/80' : 'text-ink-300')}>
                  {countFor(f.key)}
                </span>
              </span>
            </button>
          )
        })}
      </motion.div>

      {/* ── Лента ── */}
      {listQ.isLoading ? (
        <div className="space-y-3">
          {Array.from({ length: 5 }).map((_, i) => (
            <div
              key={i}
              className="h-[92px] rounded-card border border-brand-100/60 bg-surface shadow-card animate-skeleton-pulse"
            />
          ))}
        </div>
      ) : listQ.isError ? (
        <div className="rounded-card border border-brand-100/60 bg-surface p-6 shadow-card text-sm text-danger">
          Не удалось загрузить уведомления. Попробуйте обновить страницу.
        </div>
      ) : all.length === 0 ? (
        /* ── Empty state ── */
        <motion.div
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.28 }}
          className="rounded-card border border-brand-100/60 bg-surface shadow-card px-6 py-12 flex flex-col items-center text-center"
        >
          <img
            src="/empty-notifications.svg"
            alt="Нет уведомлений"
            className="w-[240px] max-w-full h-auto"
          />
          <h3 className="mt-6 text-[17px] leading-6 font-semibold text-ink-900">
            Тишина на складе
          </h3>
          <p className="mt-2 max-w-sm text-[13px] leading-[18px] text-ink-500">
            Здесь появятся передачи, напоминания о ТО и поверках, просроченные возвраты
          </p>
        </motion.div>
      ) : filtered.length === 0 ? (
        <div className="rounded-card border border-brand-100/60 bg-surface p-8 shadow-card text-center text-sm text-ink-500">
          Нет уведомлений в этой категории
        </div>
      ) : (
        <div className="space-y-6">
          {groups.map((g) => (
            <section key={g.label}>
              <h2 className="sticky top-16 lg:top-[72px] z-10 -mx-1 px-1 py-1 bg-app text-[13px] leading-[18px] font-semibold text-ink-500">
                {g.label}
              </h2>
              <div className="mt-2 space-y-3">
                <AnimatePresence initial={false}>
                  {g.items.map((n, idx) => (
                    <NotificationCard
                      key={n.id}
                      n={n}
                      index={idx}
                      onOpen={() => openNotification(n)}
                    />
                  ))}
                </AnimatePresence>
              </div>
            </section>
          ))}
        </div>
      )}

      {/* ── Тост ── */}
      <AnimatePresence>
        {toast && (
          <motion.div
            initial={{ opacity: 0, y: 24 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 12 }}
            transition={{ duration: 0.24 }}
            className="fixed bottom-24 lg:bottom-8 left-1/2 -translate-x-1/2 z-[60] inline-flex items-center gap-2 rounded-full bg-ink-900 text-white text-sm font-semibold px-5 py-3 shadow-modal"
          >
            <CheckCircle2 size={16} className="text-teal" />
            {toast}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  )
}

// ─── Карточка уведомления ────────────────────────────────────────────────────

function NotificationCard({
  n,
  index,
  onOpen,
}: {
  n: Notification
  index: number
  onOpen: () => void
}) {
  const { Icon, bg, color } = typeMeta(n)
  const created = new Date(n.createdAt)

  return (
    <motion.article
      layout="position"
      initial={{ opacity: 0, y: 16 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{
        duration: 0.26,
        delay: Math.min(index, 12) * 0.03,
        ease: [0.22, 1, 0.36, 1],
      }}
      onClick={onOpen}
      className={cn(
        'relative flex gap-3 rounded-card border border-brand-100/60 bg-surface p-4 lg:px-5 shadow-card transition-shadow duration-200',
        n.itemId ? 'cursor-pointer hover:shadow-hover' : 'cursor-default',
        !n.read && 'border-l-[3px] border-l-accent'
      )}
    >
      {/* Иконка типа */}
      <span
        className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full"
        style={{ background: bg, color }}
      >
        <Icon size={18} strokeWidth={1.75} />
      </span>

      <div className="min-w-0 flex-1">
        <div className="font-mono-num text-ink-300">
          {format(created, 'dd.MM.yyyy HH:mm')}
          {n.item?.internalId ? ` · ${n.item.internalId}` : ''}
        </div>
        <h3 className="mt-1 text-[15px] leading-[22px] font-semibold text-ink-900">
          {n.title ?? n.text}
        </h3>
        {n.title && (
          <p className="mt-0.5 text-sm leading-5 text-ink-500">{n.text}</p>
        )}
      </div>

      {/* Точка непрочитанного */}
      <AnimatePresence>
        {!n.read && (
          <motion.span
            exit={{ scale: 0, opacity: 0 }}
            transition={{ duration: 0.2 }}
            className="absolute right-4 top-4 h-2.5 w-2.5 rounded-full bg-accent"
          />
        )}
      </AnimatePresence>
    </motion.article>
  )
}
