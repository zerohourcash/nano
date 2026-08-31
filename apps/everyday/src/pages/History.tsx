import { Fragment, useEffect, useMemo, useState } from 'react'
import { Link } from 'react-router'
import {
  ArrowRight,
  CalendarDays,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Download,
  FileSpreadsheet,
  FileText,
  Network,
  RotateCcw,
  XCircle,
} from 'lucide-react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  endOfDay,
  format,
  startOfDay,
  startOfMonth,
  startOfQuarter,
  startOfWeek,
} from 'date-fns'
import { ru } from 'date-fns/locale'
import { trpc } from '@/providers/trpc'
import type { inferRouterOutputs } from '@trpc/server'
import type { AppRouter } from '../../api/router'
import { cn } from '@/lib/utils'

// ─── Типы tRPC ───────────────────────────────────────────────────────────────

type RouterOutputs = inferRouterOutputs<AppRouter>
type HistoryEntry = RouterOutputs['history']['movements'][number]

// ─── Утилиты ─────────────────────────────────────────────────────────────────

const fmtDate = (d: Date | string) => format(new Date(d), 'dd.MM.yyyy', { locale: ru })
const fmtDateTime = (d: Date | string) => format(new Date(d), 'dd.MM.yyyy HH:mm', { locale: ru })
const toInputDate = (d: Date) => format(d, 'yyyy-MM-dd')

function initials(name: string): string {
  return name
    .split(' ')
    .map((w) => w[0])
    .slice(0, 2)
    .join('')
    .toUpperCase()
}

function Avatar({ name, url, size = 20 }: { name: string; url?: string | null; size?: number }) {
  if (url) {
    return (
      <img
        src={url}
        alt={name}
        style={{ width: size, height: size }}
        className="rounded-full object-cover shrink-0 border border-brand-100"
      />
    )
  }
  return (
    <span
      style={{ width: size, height: size, fontSize: size * 0.38 }}
      className="rounded-full bg-brand-50 text-brand-600 font-semibold inline-flex items-center justify-center shrink-0 border border-brand-100"
    >
      {initials(name)}
    </span>
  )
}

/** Статус перемещения по типу записи журнала */
function movementStatus(e: HistoryEntry): { label: string; cls: string; rejected: boolean } {
  if (e.type === 'move') return { label: 'Перемещение', cls: 'bg-brand-50 text-brand-600', rejected: false }
  if (e.type === 'transfer_send')
    return { label: 'В процессе', cls: 'bg-warning-bg text-warning', rejected: false }
  if ((e.comment ?? '').toLowerCase().includes('отклон'))
    return { label: 'Отказ', cls: 'bg-danger-bg text-danger', rejected: true }
  return { label: 'Принята', cls: 'bg-success-bg text-success', rejected: false }
}

const typeLabels: Record<HistoryEntry['type'], string> = {
  move: 'Перемещение',
  transfer_send: 'Передача (отправка)',
  transfer_receive: 'Передача (приём)',
  write_off: 'Списание',
  replenish: 'Пополнение',
  inventory: 'Инвентаризация',
  create: 'Создание',
  update: 'Изменение',
}

// ─── Тост ────────────────────────────────────────────────────────────────────

function Toast({ text, tone = 'ok' }: { text: string; tone?: 'ok' | 'err' }) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 24 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, y: 12 }}
      transition={{ duration: 0.24 }}
      className="fixed bottom-24 lg:bottom-8 left-1/2 -translate-x-1/2 z-[70] inline-flex items-center gap-2 rounded-full bg-ink-900 text-white text-sm font-semibold px-5 py-3 shadow-modal max-w-[90vw]"
    >
      {tone === 'ok' ? (
        <CheckCircle2 size={16} className="text-teal shrink-0" />
      ) : (
        <XCircle size={16} className="text-danger shrink-0" />
      )}
      <span className="truncate">{text}</span>
    </motion.div>
  )
}

// ─── Мини-фото инструмента ───────────────────────────────────────────────────

function EntryPhoto({ entry }: { entry: HistoryEntry }) {
  const photos = entry.item?.photos ?? []
  const url = photos.find((p) => p.isTitle)?.url ?? photos[0]?.url ?? null
  if (url) {
    return (
      <img
        src={url}
        alt=""
        className="w-8 h-8 rounded-lg object-cover shrink-0 border border-brand-100/60 bg-brand-50"
      />
    )
  }
  return <span className="w-8 h-8 rounded-lg bg-brand-50 shrink-0 inline-block" />
}

// ─── Таблица «История перемещений» ───────────────────────────────────────────

function MovementsTable({ rows }: { rows: HistoryEntry[] }) {
  const [expanded, setExpanded] = useState<number | null>(null)

  return (
    <div className="overflow-x-auto">
      <table className="w-full min-w-[880px] text-sm">
        <thead>
          <tr className="bg-brand-50 text-left">
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold rounded-tl-xl">Дата и время</th>
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold">Вн. номер</th>
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold">Наименование</th>
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold">Маршрут</th>
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold">Автор</th>
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold">Статус</th>
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold rounded-tr-xl">Операция</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((e, i) => {
            const st = movementStatus(e)
            const actor = e.actor
            const isOpen = expanded === e.id
            return (
              <Fragment key={e.id}>
                <motion.tr
                  initial={{ opacity: 0, y: 8 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ delay: Math.min(i, 11) * 0.02, duration: 0.22 }}
                  className={cn(
                    'border-b border-brand-100/60 hover:bg-brand-50/60 transition-colors h-14',
                    st.rejected && 'cursor-pointer'
                  )}
                  onClick={() => st.rejected && setExpanded(isOpen ? null : e.id)}
                >
                  <td className="px-4 py-2 font-mono-num text-ink-900 whitespace-nowrap">
                    <span className="inline-flex items-center gap-1.5">
                      {st.rejected && (
                        <ChevronRight
                          size={13}
                          className={cn('text-ink-300 transition-transform', isOpen && 'rotate-90')}
                        />
                      )}
                      {fmtDateTime(e.createdAt)}
                    </span>
                  </td>
                  <td className="px-4 py-2">
                    {e.item ? (
                      <Link
                        to={`/tool/${e.item.id}`}
                        onClick={(ev) => ev.stopPropagation()}
                        className="font-mono-num text-brand-600 hover:underline"
                      >
                        {e.item.internalId}
                      </Link>
                    ) : (
                      <span className="font-mono-num text-ink-300">—</span>
                    )}
                  </td>
                  <td className="px-4 py-2">
                    <span className="inline-flex items-center gap-2 min-w-0">
                      <EntryPhoto entry={e} />
                      <span className="text-ink-900 font-medium truncate max-w-[220px]">
                        {e.item?.title ?? '—'}
                      </span>
                    </span>
                  </td>
                  <td className="px-4 py-2">
                    <span className="inline-flex items-center gap-1.5 text-[13px] text-ink-900 whitespace-nowrap">
                      <span className="truncate max-w-[140px]">{e.fromLabel ?? '—'}</span>
                      <ArrowRight size={13} className="text-brand-100 shrink-0" />
                      <span className="truncate max-w-[140px]">{e.toLabel ?? '—'}</span>
                    </span>
                  </td>
                  <td className="px-4 py-2">
                    {actor ? (
                      <span className="inline-flex items-center gap-1.5 text-[13px] text-ink-900 whitespace-nowrap">
                        <Avatar name={actor.fullName} url={actor.avatarUrl} />
                        <span className="truncate max-w-[110px]">{actor.fullName}</span>
                      </span>
                    ) : (
                      <span className="text-ink-300">—</span>
                    )}
                  </td>
                  <td className="px-4 py-2">
                    <motion.span
                      initial={{ scale: 0.9 }}
                      animate={{ scale: 1 }}
                      className={cn('inline-flex items-center rounded-full px-2.5 py-0.5 text-caption', st.cls)}
                    >
                      {st.label}
                    </motion.span>
                  </td>
                  <td className="px-4 py-2 font-mono-num text-[12px] text-ink-300">
                    {e.opId.slice(0, 10)}
                  </td>
                </motion.tr>
                {st.rejected && isOpen && (
                  <tr className="border-b border-brand-100/60 bg-danger-bg/30">
                    <td colSpan={7} className="px-4 py-3">
                      <div className="flex items-start gap-2 text-[13px] text-ink-900">
                        <span className="font-semibold text-danger shrink-0">Причина отказа:</span>
                        <span>{e.comment ?? '—'}</span>
                      </div>
                    </td>
                  </tr>
                )}
              </Fragment>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

// ─── Таблица «Списание и пополнение» ─────────────────────────────────────────

function QuantityTable({ rows }: { rows: HistoryEntry[] }) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full min-w-[880px] text-sm">
        <thead>
          <tr className="bg-brand-50 text-left">
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold rounded-tl-xl">Дата</th>
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold">Вн. номер</th>
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold">Наименование</th>
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold">Тип</th>
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold">Остаток</th>
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold">Автор</th>
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold">Комментарий</th>
            <th className="px-4 py-3 text-caption text-ink-500 font-semibold rounded-tr-xl">Операция</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((e, i) => {
            const replenish = e.type === 'replenish'
            const delta = e.quantityDelta
            const unit = e.item?.unit ?? 'шт'
            return (
              <motion.tr
                key={e.id}
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: Math.min(i, 11) * 0.02, duration: 0.22 }}
                className="border-b border-brand-100/60 hover:bg-brand-50/60 transition-colors h-14"
              >
                <td className="px-4 py-2 font-mono-num text-ink-900 whitespace-nowrap">
                  {fmtDate(e.createdAt)}
                </td>
                <td className="px-4 py-2">
                  {e.item ? (
                    <Link to={`/tool/${e.item.id}`} className="font-mono-num text-brand-600 hover:underline">
                      {e.item.internalId}
                    </Link>
                  ) : (
                    <span className="font-mono-num text-ink-300">—</span>
                  )}
                </td>
                <td className="px-4 py-2">
                  <span className="inline-flex items-center gap-2 min-w-0">
                    <EntryPhoto entry={e} />
                    <span className="text-ink-900 font-medium truncate max-w-[200px]">
                      {e.item?.title ?? '—'}
                    </span>
                  </span>
                </td>
                <td className="px-4 py-2">
                  <motion.span
                    initial={{ scale: 0.9 }}
                    animate={{ scale: 1 }}
                    className={cn(
                      'inline-flex items-center rounded-full px-2.5 py-0.5 text-caption whitespace-nowrap',
                      replenish ? 'bg-success-bg text-success' : 'bg-danger-bg text-danger'
                    )}
                  >
                    {replenish ? 'Пополнение' : 'Списание'}
                    {delta != null && (
                      <span className="font-mono-num ml-1">
                        {delta > 0 ? `+${delta}` : delta} {unit}
                      </span>
                    )}
                  </motion.span>
                </td>
                <td className="px-4 py-2 font-mono-num text-ink-500 whitespace-nowrap">
                  {e.item?.quantitative && e.item.quantity != null
                    ? `${e.item.quantity} ${unit}`
                    : '—'}
                </td>
                <td className="px-4 py-2">
                  {e.actor ? (
                    <span className="inline-flex items-center gap-1.5 text-[13px] text-ink-900 whitespace-nowrap">
                      <Avatar name={e.actor.fullName} url={e.actor.avatarUrl} />
                      <span className="truncate max-w-[110px]">{e.actor.fullName}</span>
                    </span>
                  ) : (
                    <span className="text-ink-300">—</span>
                  )}
                </td>
                <td className="px-4 py-2 text-[13px] text-ink-500 max-w-[220px] truncate">
                  {e.comment ?? '—'}
                </td>
                <td className="px-4 py-2 font-mono-num text-[12px] text-ink-300">
                  {e.opId.slice(0, 10)}
                </td>
              </motion.tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

// ─── Пагинация ───────────────────────────────────────────────────────────────

const PAGE_SIZE = 20

function Pagination({
  page,
  total,
  onPage,
}: {
  page: number
  total: number
  onPage: (p: number) => void
}) {
  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE))
  if (pages <= 1) return null
  const nums: (number | '…')[] = []
  for (let p = 1; p <= pages; p++) {
    if (p === 1 || p === pages || Math.abs(p - page) <= 1) nums.push(p)
    else if (nums[nums.length - 1] !== '…') nums.push('…')
  }
  return (
    <div className="flex items-center justify-between gap-3 flex-wrap px-4 py-3">
      <span className="text-[13px] text-ink-500">
        Показано{' '}
        <span className="font-mono-num text-ink-900">
          {Math.min(PAGE_SIZE, total - (page - 1) * PAGE_SIZE)}
        </span>{' '}
        из <span className="font-mono-num text-ink-900">{total}</span>
      </span>
      <div className="flex items-center gap-1">
        <button
          onClick={() => onPage(page - 1)}
          disabled={page <= 1}
          className="w-8 h-8 rounded-lg inline-flex items-center justify-center text-ink-500 hover:bg-brand-50 disabled:opacity-40"
          aria-label="Назад"
        >
          ‹
        </button>
        {nums.map((n, i) =>
          n === '…' ? (
            <span key={`e${i}`} className="w-8 text-center text-ink-300 text-sm">
              …
            </span>
          ) : (
            <button
              key={n}
              onClick={() => onPage(n)}
              className={cn(
                'w-8 h-8 text-sm font-semibold transition-colors',
                n === page
                  ? 'rounded-full bg-brand-600 text-white'
                  : 'rounded-lg text-ink-500 hover:bg-brand-50'
              )}
            >
              {n}
            </button>
          )
        )}
        <button
          onClick={() => onPage(page + 1)}
          disabled={page >= pages}
          className="w-8 h-8 rounded-lg inline-flex items-center justify-center text-ink-500 hover:bg-brand-50 disabled:opacity-40"
          aria-label="Вперёд"
        >
          ›
        </button>
      </div>
    </div>
  )
}

// ─── Страница ────────────────────────────────────────────────────────────────

type RangeKey = 'today' | 'week' | 'month' | 'quarter' | null

export default function History() {
  const [tab, setTab] = useState<'movements' | 'quantity'>('movements')
  const [dateFrom, setDateFrom] = useState('')
  const [dateTo, setDateTo] = useState('')
  const [range, setRange] = useState<RangeKey>(null)
  const [authorId, setAuthorId] = useState<number | null>(null)
  const [place, setPlace] = useState<string | null>(null)
  const [page, setPage] = useState(1)
  const [exportOpen, setExportOpen] = useState(false)
  const [toast, setToast] = useState<{ text: string; tone: 'ok' | 'err' } | null>(null)

  useEffect(() => {
    if (!toast) return
    const t = setTimeout(() => setToast(null), 4000)
    return () => clearTimeout(t)
  }, [toast])

  // Сброс страницы при смене фильтров/таба
  useEffect(() => {
    const frame = requestAnimationFrame(() => setPage(1))
    return () => cancelAnimationFrame(frame)
  }, [tab, dateFrom, dateTo, authorId, place])

  const queryInput = useMemo(() => {
    const from = dateFrom ? startOfDay(new Date(`${dateFrom}T00:00:00`)) : undefined
    const to = dateTo ? endOfDay(new Date(`${dateTo}T00:00:00`)) : undefined
    return { limit: 500, dateFrom: from, dateTo: to }
  }, [dateFrom, dateTo])

  const movementsQ = trpc.history.movements.useQuery(queryInput)
  const quantityQ = trpc.history.quantityOps.useQuery(queryInput)
  const allQ = trpc.history.all.useQuery({ limit: 500 })
  const storagesQ = trpc.admin.storages.list.useQuery({})
  const sitesQ = trpc.admin.buildingSites.list.useQuery({})

  // Список авторов из загруженного журнала
  const authors = useMemo(() => {
    const map = new Map<number, { id: number; fullName: string; avatarUrl: string | null }>()
    for (const e of allQ.data ?? []) {
      if (e.actor && !map.has(e.actor.id)) {
        map.set(e.actor.id, { id: e.actor.id, fullName: e.actor.fullName, avatarUrl: e.actor.avatarUrl })
      }
    }
    return [...map.values()].sort((a, b) => a.fullName.localeCompare(b.fullName, 'ru'))
  }, [allQ.data])

  const places = useMemo(() => {
    const list: string[] = []
    for (const s of storagesQ.data ?? []) list.push(s.name)
    for (const s of sitesQ.data ?? []) list.push(s.name)
    return list
  }, [storagesQ.data, sitesQ.data])

  const applyClientFilters = (rows: HistoryEntry[]) =>
    rows.filter((e) => {
      if (authorId != null && e.actorUserId !== authorId) return false
      if (place && e.fromLabel !== place && e.toLabel !== place) return false
      return true
    })

  const movements = useMemo(
    () => applyClientFilters(movementsQ.data ?? []),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [movementsQ.data, authorId, place]
  )
  const quantityOps = useMemo(
    () => applyClientFilters(quantityQ.data ?? []),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [quantityQ.data, authorId, place]
  )

  const activeRows = tab === 'movements' ? movements : quantityOps
  const pagedRows = activeRows.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE)
  const loading = tab === 'movements' ? movementsQ.isLoading : quantityQ.isLoading
  const loadError = tab === 'movements' ? movementsQ.error : quantityQ.error

  const setQuickRange = (key: Exclude<RangeKey, null>) => {
    const now = new Date()
    let from: Date
    if (key === 'today') from = startOfDay(now)
    else if (key === 'week') from = startOfWeek(now, { locale: ru })
    else if (key === 'month') from = startOfMonth(now)
    else from = startOfQuarter(now)
    setRange(key)
    setDateFrom(toInputDate(from))
    setDateTo(toInputDate(now))
  }

  const resetFilters = () => {
    setDateFrom('')
    setDateTo('')
    setRange(null)
    setAuthorId(null)
    setPlace(null)
  }

  // ─── Экспорт CSV ───
  const exportCsv = () => {
    const rows = allQ.data ?? []
    const header = [
      'ID',
      'Дата и время',
      'Тип',
      'Вн. номер',
      'Наименование',
      'Откуда',
      'Куда',
      'Изменение кол-ва',
      'Автор',
      'Комментарий',
      'ID операции',
    ]
    const esc = (v: unknown) => {
      const s = v == null ? '' : String(v)
      return /[";\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s
    }
    const lines = rows.map((e) =>
      [
        e.id,
        fmtDateTime(e.createdAt),
        typeLabels[e.type],
        e.item?.internalId ?? '',
        e.item?.title ?? '',
        e.fromLabel ?? '',
        e.toLabel ?? '',
        e.quantityDelta ?? '',
        e.actor?.fullName ?? '',
        e.comment ?? '',
        e.opId,
      ]
        .map(esc)
        .join(';')
    )
    const csv = '﻿' + [header.map(esc).join(';'), ...lines].join('\n')
    const blob = new Blob([csv], { type: 'text/csv;charset=utf-8' })
    const a = document.createElement('a')
    a.href = URL.createObjectURL(blob)
    a.download = `meshkeeper-journal-${toInputDate(new Date())}.csv`
    a.click()
    URL.revokeObjectURL(a.href)
    setToast({
      text: 'Журнал выгружен. В офлайн-версии этот файл станет снапшотом локальной ноды',
      tone: 'ok',
    })
  }

  const tabs = [
    { key: 'movements' as const, label: 'История перемещений', count: movements.length },
    { key: 'quantity' as const, label: 'Списание и пополнение', count: quantityOps.length },
  ]

  const chips: { key: Exclude<RangeKey, null>; label: string }[] = [
    { key: 'today', label: 'Сегодня' },
    { key: 'week', label: 'Неделя' },
    { key: 'month', label: 'Месяц' },
    { key: 'quarter', label: 'Квартал' },
  ]

  return (
    <div className="space-y-5">
      {/* ─── Секция 1. Заголовок ─── */}
      <motion.div
        initial={{ opacity: 0, y: 16 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.28 }}
        className="flex flex-wrap items-start justify-between gap-3"
      >
        <div>
          <h1 className="text-2xl lg:text-[28px] leading-9 font-bold tracking-[-0.01em] text-ink-900">
            История
          </h1>
          <p className="text-[13px] text-ink-500 mt-1">
            Журнал операций ·{' '}
            <span className="font-mono-num">{allQ.data?.length ?? '—'}</span> записей ·
            неизменяемый реестр
          </p>
        </div>
        <div className="relative">
          <button
            onClick={() => setExportOpen((v) => !v)}
            className="h-10 px-4 rounded-lg border border-brand-100 bg-surface text-sm font-semibold text-ink-900 hover:bg-brand-50 transition-colors inline-flex items-center gap-2"
          >
            <Download size={16} className="text-brand-600" />
            Экспорт журнала
            <ChevronDown size={14} className={cn('text-ink-500 transition-transform', exportOpen && 'rotate-180')} />
          </button>
          <AnimatePresence>
            {exportOpen && (
              <>
                <div className="fixed inset-0 z-40" onClick={() => setExportOpen(false)} />
                <motion.div
                  initial={{ opacity: 0, y: -6, scale: 0.98 }}
                  animate={{ opacity: 1, y: 0, scale: 1 }}
                  exit={{ opacity: 0, y: -6, scale: 0.98 }}
                  transition={{ duration: 0.15 }}
                  className="absolute right-0 top-12 z-50 w-48 bg-surface rounded-xl border border-brand-100/60 shadow-hover p-1.5"
                >
                  <button
                    onClick={() => {
                      setExportOpen(false)
                      exportCsv()
                    }}
                    className="w-full flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm font-medium text-ink-900 hover:bg-brand-50 transition-colors"
                  >
                    <FileSpreadsheet size={15} className="text-success" />
                    CSV
                  </button>
                  <button
                    onClick={() => {
                      setExportOpen(false)
                      setToast({
                        text: 'Журнал выгружен. В офлайн-версии этот файл станет снапшотом локальной ноды',
                        tone: 'ok',
                      })
                    }}
                    className="w-full flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm font-medium text-ink-900 hover:bg-brand-50 transition-colors"
                  >
                    <FileText size={15} className="text-danger" />
                    PDF
                  </button>
                </motion.div>
              </>
            )}
          </AnimatePresence>
        </div>
      </motion.div>

      {/* ─── Секция 2. Табы ─── */}
      <div className="flex border-b border-brand-100/60 gap-2 sm:gap-6">
        {tabs.map((t) => (
          <button
            key={t.key}
            onClick={() => setTab(t.key)}
            className={cn(
              'relative flex-1 sm:flex-none pb-3 pt-1 px-2 text-base font-semibold transition-colors',
              tab === t.key ? 'text-brand-600' : 'text-ink-500 hover:text-ink-900'
            )}
          >
            {t.label}{' '}
            <span className={cn('font-mono-num', tab === t.key ? 'text-accent' : 'text-ink-500')}>
              ({t.count})
            </span>
            {tab === t.key && (
              <motion.span
                layoutId="history-tab-underline"
                className="absolute left-0 right-0 -bottom-px h-0.5 bg-accent rounded-full"
                transition={{ duration: 0.2 }}
              />
            )}
          </button>
        ))}
      </div>

      {/* ─── Секция 3. Панель фильтров журнала ─── */}
      <div className="bg-surface rounded-card border border-brand-100/60 shadow-card px-4 py-3 flex flex-wrap items-center gap-3">
        <div className="flex items-center gap-2 flex-wrap">
          <CalendarDays size={16} className="text-ink-500" />
          <input
            type="date"
            value={dateFrom}
            onChange={(e) => {
              setDateFrom(e.target.value)
              setRange(null)
            }}
            className="h-9 rounded-lg border border-brand-100 bg-surface px-3 text-[13px] font-mono-num text-ink-900 focus:border-brand-600 focus:outline-none"
            aria-label="Дата от"
          />
          <span className="text-ink-300">—</span>
          <input
            type="date"
            value={dateTo}
            onChange={(e) => {
              setDateTo(e.target.value)
              setRange(null)
            }}
            className="h-9 rounded-lg border border-brand-100 bg-surface px-3 text-[13px] font-mono-num text-ink-900 focus:border-brand-600 focus:outline-none"
            aria-label="Дата до"
          />
        </div>
        <div className="flex items-center gap-1.5 flex-wrap">
          {chips.map((c) => (
            <button
              key={c.key}
              onClick={() => setQuickRange(c.key)}
              className={cn(
                'h-9 px-3 rounded-full text-[13px] font-semibold transition-colors',
                range === c.key ? 'bg-brand-600 text-white' : 'bg-brand-50 text-ink-500 hover:text-ink-900'
              )}
            >
              {c.label}
            </button>
          ))}
        </div>
        <select
          value={authorId ?? ''}
          onChange={(e) => setAuthorId(e.target.value ? Number(e.target.value) : null)}
          className="h-9 rounded-lg border border-brand-100 bg-surface px-3 text-[13px] text-ink-900 focus:border-brand-600 focus:outline-none max-w-[180px]"
          aria-label="Автор"
        >
          <option value="">Все авторы</option>
          {authors.map((a) => (
            <option key={a.id} value={a.id}>
              {a.fullName}
            </option>
          ))}
        </select>
        <select
          value={place ?? ''}
          onChange={(e) => setPlace(e.target.value || null)}
          className="h-9 rounded-lg border border-brand-100 bg-surface px-3 text-[13px] text-ink-900 focus:border-brand-600 focus:outline-none max-w-[200px]"
          aria-label="Объект или склад"
        >
          <option value="">Все объекты и склады</option>
          {places.map((p) => (
            <option key={p} value={p}>
              {p}
            </option>
          ))}
        </select>
        <button
          onClick={resetFilters}
          className="inline-flex items-center gap-1.5 text-[13px] font-semibold text-brand-600 hover:bg-brand-50 rounded-lg px-3 h-9 transition-colors"
        >
          <RotateCcw size={14} />
          Сбросить
        </button>
      </div>

      {/* ─── Секции 4–5. Таблица ─── */}
      <AnimatePresence mode="wait">
        <motion.div
          key={tab + dateFrom + dateTo + (authorId ?? '') + (place ?? '')}
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.12 }}
          className="bg-surface rounded-card border border-brand-100/60 shadow-card overflow-hidden"
        >
          {loading ? (
            <div className="p-4 space-y-3 animate-pulse">
              {[0, 1, 2].map((i) => (
                <div key={i} className="h-12 rounded-lg bg-brand-50" />
              ))}
            </div>
          ) : loadError ? (
            <div className="p-6 text-sm text-danger flex items-center justify-between gap-3">
              <span>Не удалось загрузить журнал: {loadError.message}</span>
              <button
                onClick={() => (tab === 'movements' ? movementsQ.refetch() : quantityQ.refetch())}
                className="h-9 px-4 rounded-lg border border-brand-100 text-ink-900 font-semibold hover:bg-brand-50"
              >
                Повторить
              </button>
            </div>
          ) : activeRows.length === 0 ? (
            <div className="px-6 py-14 flex flex-col items-center text-center gap-2">
              <Network size={32} className="text-brand-100" />
              <h3 className="text-[17px] font-semibold text-ink-900">Записей не найдено</h3>
              <p className="text-[13px] text-ink-500 max-w-[320px]">
                Попробуйте изменить фильтры — журнал хранит все операции рабочего пространства
              </p>
            </div>
          ) : (
            <>
              {tab === 'movements' ? (
                <MovementsTable rows={pagedRows} />
              ) : (
                <QuantityTable rows={pagedRows} />
              )}
              <Pagination page={page} total={activeRows.length} onPage={setPage} />
            </>
          )}
        </motion.div>
      </AnimatePresence>

      {/* ─── Секция 7. Инфо-баннер про леджер ─── */}
      <motion.div
        initial={{ opacity: 0, y: 20 }}
        whileInView={{ opacity: 1, y: 0 }}
        viewport={{ once: true, margin: '-40px' }}
        transition={{ duration: 0.3 }}
        className="flex items-start gap-3 rounded-card bg-info-bg border-l-[3px] border-teal px-5 py-4"
      >
        <motion.span
          animate={{ scale: [1, 1.12, 1] }}
          transition={{ duration: 3, repeat: Infinity, ease: 'easeInOut' }}
          className="text-teal-dark shrink-0 mt-0.5"
        >
          <Network size={18} />
        </motion.span>
        <p className="text-sm text-ink-900 leading-snug flex-1">
          Этот журнал — неизменяемый реестр операций. В офлайн-версии Everyday каждая запись
          станет блоком локальной цепочки устройства (модель Nano), а синхронизация по mesh-сети
          сведёт журналы всех узлов без интернета.
        </p>
        <button
          onClick={() => setToast({ text: 'Функция в разработке', tone: 'ok' })}
          className="shrink-0 rounded-full bg-teal/20 text-teal-dark text-[12px] font-semibold px-3 py-1.5 hover:bg-teal/30 transition-colors"
        >
          Подробнее о дорожной карте
        </button>
      </motion.div>

      {/* ─── Тост ─── */}
      <AnimatePresence>{toast && <Toast text={toast.text} tone={toast.tone} />}</AnimatePresence>
    </div>
  )
}
