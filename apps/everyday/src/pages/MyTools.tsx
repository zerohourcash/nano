import { useEffect, useMemo, useRef, useState } from 'react'
import { Link, useNavigate } from 'react-router'
import {
  Search,
  X,
  SlidersHorizontal,
  LayoutGrid,
  List,
  Plus,
  Check,
  ArrowUpDown,
  ArrowUp,
  ArrowDown,
  MoreVertical,
  QrCode,
  Trash2,
  ArrowLeftRight,
  Loader2,
  CheckCircle2,
  Pin,
  Package,
  Inbox,
  Send,
  AlarmClock,
  Building2,
  Warehouse,
  Phone,
  Wrench,
} from 'lucide-react'
import { AnimatePresence, motion } from 'framer-motion'
import type { inferRouterOutputs } from '@trpc/server'
import type { AppRouter } from '../../api/router'
import { trpc } from '@/providers/trpc'
import { askStatusReason, statusNeedsReason } from '@/lib/status-reason'
import { cn } from '@/lib/utils'
import { QrBadge, MaterialBadge } from '@/components/StatusBadge'

// ─── Типы из tRPC-контракта ─────────────────────────────────────────────────

type RouterOutputs = inferRouterOutputs<AppRouter>
type Item = RouterOutputs['items']['list']['rows'][number]
type Transfer = RouterOutputs['transfers']['incoming'][number]
type SortKey = 'createdAt_desc' | 'createdAt_asc' | 'title_asc' | 'title_desc' | 'internalId_asc'

const sortOptions: { key: SortKey; label: string }[] = [
  { key: 'createdAt_desc', label: 'Сначала новые' },
  { key: 'createdAt_asc', label: 'Сначала старые' },
  { key: 'title_asc', label: 'По названию А–Я' },
  { key: 'title_desc', label: 'По названию Я–А' },
  { key: 'internalId_asc', label: 'По вн. номеру' },
]

interface Filters {
  sites: number[]
  warehouses: number[]
  categories: number[]
  brands: number[]
  statuses: number[]
  qr: 'with' | 'without' | null
}

const emptyFilters: Filters = { sites: [], warehouses: [], categories: [], brands: [], statuses: [], qr: null }

const countFilters = (f: Filters) =>
  f.sites.length + f.warehouses.length + f.categories.length + f.brands.length + f.statuses.length + (f.qr ? 1 : 0)

type ViewMode = 'grid' | 'table'
const PAGE_SIZE = 8
const FETCH_LIMIT = 100

// ─── Утилиты ─────────────────────────────────────────────────────────────────

const titlePhoto = (item: Item) => item.photos.find((p) => p.isTitle) ?? item.photos[0]

function applyFilters(list: Item[], f: Filters): Item[] {
  return list.filter((t) => {
    if (f.sites.length && !(t.buildingSiteId && f.sites.includes(t.buildingSiteId))) return false
    if (f.warehouses.length && !(t.storageId && f.warehouses.includes(t.storageId))) return false
    if (f.categories.length && !(t.categoryId && f.categories.includes(t.categoryId))) return false
    if (f.brands.length && !(t.brandId && f.brands.includes(t.brandId))) return false
    if (f.statuses.length && !(t.statusId && f.statuses.includes(t.statusId))) return false
    if (f.qr === 'with' && !t.qrCode) return false
    if (f.qr === 'without' && t.qrCode) return false
    return true
  })
}

/** Подсветка совпадений поиска (#FBFCC8) */
function Highlight({ text, query }: { text: string; query: string }) {
  const q = query.trim()
  if (!q) return <>{text}</>
  const idx = text.toLowerCase().indexOf(q.toLowerCase())
  if (idx === -1) return <>{text}</>
  return (
    <>
      {text.slice(0, idx)}
      <mark className="bg-warning-bg rounded-sm px-0.5">{text.slice(idx, idx + q.length)}</mark>
      {text.slice(idx + q.length)}
    </>
  )
}

/** Count-up анимация числа (0→N, 600ms, ease-out) при первом появлении */
function CountUp({ value, className }: { value: number; className?: string }) {
  const [display, setDisplay] = useState(0)
  useEffect(() => {
    let raf = 0
    const start = performance.now()
    const tick = (t: number) => {
      const p = Math.min(1, (t - start) / 600)
      setDisplay(Math.round((1 - Math.pow(1 - p, 3)) * value))
      if (p < 1) raf = requestAnimationFrame(tick)
    }
    raf = requestAnimationFrame(tick)
    return () => cancelAnimationFrame(raf)
  }, [value])
  return <span className={className}>{display}</span>
}

/** Цветная точка + подпись статуса из справочника (color из API) */
function ItemStatusDot({ item, className }: { item: Item; className?: string }) {
  const s = item.status
  if (!s) return <span className={cn('text-xs text-ink-300', className)}>—</span>
  return (
    <span className={cn('inline-flex items-center gap-1.5 text-xs font-semibold', className)} style={{ color: s.color }}>
      <span className="w-1.5 h-1.5 rounded-full shrink-0" style={{ background: s.color }} />
      {s.name}
    </span>
  )
}

/** Статусный бейдж pill (uppercase, цветная пара фон/текст из API) */
function ItemStatusBadge({ item, className }: { item: Item; className?: string }) {
  const s = item.status
  if (!s) return <span className={cn('text-xs text-ink-300', className)}>—</span>
  return (
    <span
      className={cn('inline-flex items-center rounded-full px-2.5 py-0.5 text-caption', className)}
      style={{ background: s.bg, color: s.color }}
    >
      {s.name}
    </span>
  )
}

// ─── Чекбокс фильтра ─────────────────────────────────────────────────────────

function FilterCheckbox({
  checked,
  onChange,
  label,
  count,
  dot,
}: {
  checked: boolean
  onChange: () => void
  label: string
  count: number
  dot?: string
}) {
  return (
    <button
      onClick={onChange}
      className="flex items-center gap-2.5 rounded-lg px-2 py-1.5 hover:bg-brand-50/70 transition-colors text-left min-w-0"
    >
      <span
        className={cn(
          'w-5 h-5 shrink-0 rounded-md border-[1.5px] flex items-center justify-center transition-colors duration-150',
          checked ? 'bg-brand-600 border-brand-600' : 'border-brand-100 bg-white'
        )}
      >
        {checked && (
          <motion.svg initial={{ opacity: 0 }} animate={{ opacity: 1 }} width="12" height="12" viewBox="0 0 12 12" fill="none">
            <motion.path
              d="M2 6.2L4.8 9L10 3"
              stroke="white"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              initial={{ pathLength: 0 }}
              animate={{ pathLength: 1 }}
              transition={{ duration: 0.15 }}
            />
          </motion.svg>
        )}
      </span>
      {dot && <span className="w-2 h-2 rounded-full shrink-0" style={{ background: dot }} />}
      <span className="text-sm text-ink-900 truncate flex-1">{label}</span>
      <span className="font-mono-num text-ink-300">({count})</span>
    </button>
  )
}

// ─── Скелетон карточки ───────────────────────────────────────────────────────

function CardSkeleton() {
  return (
    <div className="bg-surface rounded-mini border border-brand-100/60 shadow-card p-3">
      <div className="aspect-[4/3] rounded-[10px] animate-skeleton-pulse" />
      <div className="pt-2.5 space-y-2">
        <div className="h-3 w-1/3 rounded animate-skeleton-pulse" />
        <div className="h-4 w-full rounded animate-skeleton-pulse" />
        <div className="h-3 w-2/3 rounded animate-skeleton-pulse" />
        <div className="h-6 w-full rounded animate-skeleton-pulse" />
      </div>
    </div>
  )
}

// ─── Мини-карточка инструмента (мои инструменты) ────────────────────────────

function MyToolCard({
  item,
  query,
  selected,
  selectionMode,
  incoming,
  onToggleSelect,
  onAcceptClick,
  onCallClick,
}: {
  item: Item
  query: string
  selected: boolean
  selectionMode: boolean
  /** входящая передача по этому инструменту (бейдж «Приёмка») */
  incoming?: Transfer
  onToggleSelect: () => void
  onAcceptClick: (t: Transfer) => void
  onCallClick: (item: Item) => void
}) {
  const navigate = useNavigate()
  const photo = titlePhoto(item)
  const assignee = item.responsible

  return (
    <motion.article
      onClick={() => navigate(`/tool/${item.id}`)}
      whileHover={{ y: -2 }}
      transition={{ duration: 0.2, ease: 'easeOut' }}
      className={cn(
        'group relative bg-surface rounded-mini border shadow-card p-3 cursor-pointer transition-shadow hover:shadow-hover',
        selected ? 'border-brand-600 ring-2 ring-brand-600/20' : 'border-brand-100/60'
      )}
    >
      {/* Фото 4:3 + чекбокс + бейджи */}
      <div className="relative overflow-hidden rounded-[10px] aspect-[4/3] bg-brand-50">
        {photo ? (
          <img
            src={photo.url}
            alt={item.title}
            loading="lazy"
            className="w-full h-full object-cover transition-transform duration-300 group-hover:scale-[1.03]"
          />
        ) : (
          <div className="w-full h-full flex items-center justify-center text-brand-100">
            <Wrench size={40} strokeWidth={1.5} />
          </div>
        )}
        <button
          onClick={(e) => {
            e.stopPropagation()
            onToggleSelect()
          }}
          aria-label={selected ? 'Снять выбор' : 'Выбрать'}
          className={cn(
            'absolute left-2 top-2 w-5 h-5 rounded-md border-[1.5px] flex items-center justify-center transition-all duration-150',
            'bg-white/80 backdrop-blur-sm',
            selected
              ? 'bg-brand-600 border-brand-600 opacity-100'
              : 'border-brand-100 opacity-0 group-hover:opacity-100',
            (selectionMode || selected) && 'opacity-100'
          )}
        >
          {selected && (
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
              <path d="M2 6.2L4.8 9L10 3" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          )}
        </button>
        {/* Бейдж входящей приёмки (пульсирует) */}
        {incoming && (
          <motion.button
            onClick={(e) => {
              e.stopPropagation()
              onAcceptClick(incoming)
            }}
            animate={{ scale: [1, 1.06, 1] }}
            transition={{ duration: 2.5, repeat: Infinity, ease: 'easeInOut' }}
            className="absolute right-2 top-2 inline-flex items-center gap-1 rounded-full bg-warning-bg px-2 py-0.5 text-[11px] font-semibold text-ink-900 shadow-card"
          >
            <Inbox size={11} strokeWidth={2} />
            Приёмка
          </motion.button>
        )}
        {(item.qrCode || item.quantitative) && (
          <div className="absolute right-2 bottom-2 flex gap-1">
            {item.qrCode && <QrBadge />}
            {item.quantitative && <MaterialBadge />}
          </div>
        )}
      </div>

      {/* Тело карточки */}
      <div className="pt-2.5 space-y-1.5">
        <div className="flex items-center justify-between gap-2">
          <span className="font-mono-num text-ink-500">
            <Highlight text={item.internalId} query={query} />
          </span>
          <ItemStatusDot item={item} />
        </div>
        <h3 className="text-[15px] leading-[22px] font-semibold text-ink-900 line-clamp-2 min-h-[44px]">
          <Highlight text={item.title} query={query} />
        </h3>

        {/* Мета: объект · склад */}
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-ink-500">
          {item.buildingSite && (
            <span className="inline-flex items-center gap-1 min-w-0">
              <Building2 size={13} strokeWidth={1.75} className="shrink-0 text-ink-300" />
              <span className="truncate">{item.buildingSite.name}</span>
            </span>
          )}
          {item.storage && (
            <span className="inline-flex items-center gap-1 min-w-0">
              <Warehouse size={13} strokeWidth={1.75} className="shrink-0 text-ink-300" />
              <span className="truncate">{item.storage.name}</span>
            </span>
          )}
          {item.quantitative && typeof item.quantity === 'number' && (
            <span className="font-mono-num text-ink-500">
              {item.quantity} {item.unit ?? 'шт'}
            </span>
          )}
        </div>

        {/* Ответственный */}
        <div className="flex items-center gap-2 pt-1 border-t border-brand-100/60">
          {assignee ? (
            <>
              {assignee.avatarUrl ? (
                <img
                  src={assignee.avatarUrl}
                  alt={assignee.fullName}
                  className="w-5 h-5 rounded-full object-cover border border-brand-100"
                />
              ) : (
                <span className="w-5 h-5 rounded-full bg-brand-50 border border-brand-100 flex items-center justify-center text-[10px] font-semibold text-brand-600">
                  {assignee.fullName.slice(0, 1)}
                </span>
              )}
              <span className="text-xs font-semibold text-ink-900 truncate flex-1">{assignee.fullName}</span>
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  onCallClick(item)
                  window.location.href = `tel:${assignee.phone.replace(/[^+\d]/g, '')}`
                }}
                aria-label={`Позвонить: ${assignee.fullName}`}
                className="w-8 h-8 shrink-0 flex items-center justify-center rounded-lg text-teal-dark hover:bg-teal/15 transition-colors"
              >
                <Phone size={15} strokeWidth={1.75} />
              </button>
            </>
          ) : (
            <span className="text-xs text-ink-300 py-1.5">Без ответственного</span>
          )}
        </div>
      </div>
    </motion.article>
  )
}

// ─── Модалка приёмки входящей передачи ───────────────────────────────────────

function AcceptModal({
  transfer,
  onClose,
  onToast,
}: {
  transfer: Transfer | null
  onClose: () => void
  onToast: (msg: string) => void
}) {
  const utils = trpc.useUtils()
  const [comment, setComment] = useState('')

  useEffect(() => {
    const frame = requestAnimationFrame(() => setComment(''))
    return () => cancelAnimationFrame(frame)
  }, [transfer?.id])

  const invalidate = () => {
    utils.transfers.incoming.invalidate()
    utils.transfers.outgoing.invalidate()
    utils.meta.transferCounts.invalidate()
    utils.items.list.invalidate()
  }
  const accept = trpc.transfers.accept.useMutation({
    onSuccess: () => {
      invalidate()
      onToast(`Передача ${transfer?.code ?? ''} принята`)
      onClose()
    },
    onError: (e) => onToast(e.message),
  })
  const reject = trpc.transfers.reject.useMutation({
    onSuccess: () => {
      invalidate()
      onToast(`Передача ${transfer?.code ?? ''} отклонена`)
      onClose()
    },
    onError: (e) => onToast(e.message),
  })
  const busy = accept.isPending || reject.isPending

  return (
    <AnimatePresence>
      {transfer && (
        <>
          <motion.div
            key="accept-overlay"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.16 }}
            onClick={() => !busy && onClose()}
            className="fixed inset-0 z-50 bg-[rgba(48,52,102,.45)] backdrop-blur-sm"
          />
          <motion.div
            key="accept-modal"
            initial={{ opacity: 0, scale: 0.96 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.96 }}
            transition={{ type: 'spring', stiffness: 300, damping: 26 }}
            className="fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 z-50 w-[calc(100vw-32px)] max-w-[480px] bg-surface rounded-modal shadow-modal p-5 sm:p-6"
          >
            <div className="flex items-start justify-between gap-3">
              <div>
                <h3 className="text-[17px] leading-6 font-semibold text-ink-900">Приёмка инструмента</h3>
                <span className="font-mono-num text-ink-500">{transfer.code}</span>
              </div>
              <button
                onClick={() => !busy && onClose()}
                className="w-9 h-9 flex items-center justify-center rounded-xl text-ink-300 hover:bg-brand-50 hover:text-ink-900 transition-colors"
                aria-label="Закрыть"
              >
                <X size={17} />
              </button>
            </div>

            <div className="mt-4 flex items-center gap-3 rounded-xl border border-brand-100/70 bg-brand-50/50 p-3">
              {(() => {
                const photo = transfer.item ? titlePhoto(transfer.item as Item) : undefined
                return photo ? (
                  <img src={photo.url} alt="" className="w-14 h-14 rounded-lg object-cover border border-brand-100/60" />
                ) : (
                  <span className="w-14 h-14 rounded-lg bg-brand-50 border border-brand-100/60 flex items-center justify-center text-brand-100">
                    <Wrench size={22} strokeWidth={1.5} />
                  </span>
                )
              })()}
              <div className="min-w-0">
                <div className="font-mono-num text-ink-500">{transfer.item?.internalId}</div>
                <div className="text-sm font-semibold text-ink-900 line-clamp-1">{transfer.item?.title}</div>
                <div className="text-xs text-ink-500 mt-0.5">
                  От: {transfer.fromUser?.fullName ?? '—'}
                  {typeof transfer.quantity === 'number' && ` · ${transfer.quantity} шт`}
                </div>
              </div>
            </div>

            <label className="block mt-4">
              <span className="text-[13px] font-semibold text-ink-500">Комментарий</span>
              <textarea
                value={comment}
                onChange={(e) => setComment(e.target.value)}
                rows={2}
                placeholder="Состояние, замечания…"
                className="mt-1.5 w-full rounded-xl border border-brand-100 bg-white px-3.5 py-2.5 text-sm text-ink-900 placeholder:text-ink-300 focus:border-brand-600 focus:shadow-[0_0_0_3px_#5E629B22] transition-shadow resize-none"
              />
            </label>

            <div className="mt-5 flex justify-end gap-2">
              <button
                onClick={() => reject.mutate({ id: transfer.id, comment: comment || undefined })}
                disabled={busy}
                className="inline-flex items-center gap-1.5 h-10 px-4 rounded-xl border border-danger text-sm font-semibold text-danger hover:bg-danger-bg disabled:opacity-60 transition"
              >
                {reject.isPending && <Loader2 size={15} className="animate-spin" />}
                Отклонить
              </button>
              <button
                onClick={() => accept.mutate({ id: transfer.id, comment: comment || undefined })}
                disabled={busy}
                className="inline-flex items-center gap-1.5 h-10 px-5 rounded-xl bg-accent text-white text-sm font-semibold hover:bg-accent-hover hover:-translate-y-px active:scale-[0.97] disabled:opacity-60 transition"
              >
                {accept.isPending && <Loader2 size={15} className="animate-spin" />}
                Принять
              </button>
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  )
}

// ─── Табличный вид ───────────────────────────────────────────────────────────

function MyToolsTable({
  items,
  query,
  selectedIds,
  selectionMode,
  onSelectRow,
  onDelete,
}: {
  items: Item[]
  query: string
  selectedIds: Set<number>
  selectionMode: boolean
  onSelectRow: (id: number, checked: boolean) => void
  onDelete: (item: Item) => void
}) {
  const navigate = useNavigate()
  const [menuFor, setMenuFor] = useState<number | null>(null)
  const [sortCol, setSortCol] = useState<string | null>(null)
  const [sortDir, setSortDir] = useState<'asc' | 'desc'>('asc')

  const sorted = useMemo(() => {
    if (!sortCol) return items
    const arr = [...items]
    const dir = sortDir === 'asc' ? 1 : -1
    switch (sortCol) {
      case 'vn':
        return arr.sort((a, b) => a.internalId.localeCompare(b.internalId, 'ru') * dir)
      case 'name':
        return arr.sort((a, b) => a.title.localeCompare(b.title, 'ru') * dir)
      case 'category':
        return arr.sort((a, b) => (a.category?.name ?? '').localeCompare(b.category?.name ?? '', 'ru') * dir)
      case 'qty':
        return arr.sort((a, b) => ((a.quantity ?? 0) - (b.quantity ?? 0)) * dir)
      default:
        return arr
    }
  }, [items, sortCol, sortDir])

  const headerCell = (key: string, label: string, sortable = true) => (
    <th
      onClick={() => {
        if (!sortable) return
        if (sortCol === key) setSortDir((d) => (d === 'asc' ? 'desc' : 'asc'))
        else {
          setSortCol(key)
          setSortDir('asc')
        }
      }}
      className={cn(
        'px-3 py-3 text-left text-caption text-ink-500 bg-brand-50 whitespace-nowrap select-none',
        sortable && 'cursor-pointer hover:text-ink-900'
      )}
    >
      <span className="inline-flex items-center gap-1">
        {label}
        {sortable &&
          (sortCol === key ? (
            <motion.span animate={{ rotate: sortDir === 'asc' ? 0 : 180 }} transition={{ duration: 0.18 }}>
              <ArrowUp size={12} />
            </motion.span>
          ) : (
            <ArrowDown size={12} className="opacity-40" />
          ))}
      </span>
    </th>
  )

  const allChecked = items.length > 0 && items.every((t) => selectedIds.has(t.id))

  return (
    <div className="overflow-x-auto">
      <table className="w-full min-w-[960px] border-collapse">
        <thead className="sticky top-0 z-10">
          <tr>
            <th className="w-10 px-3 py-3 bg-brand-50">
              <button
                onClick={() => items.forEach((t) => onSelectRow(t.id, !allChecked))}
                className={cn(
                  'w-5 h-5 rounded-md border-[1.5px] flex items-center justify-center transition-colors',
                  allChecked ? 'bg-brand-600 border-brand-600' : 'border-brand-100 bg-white'
                )}
              >
                {allChecked && <Check size={13} className="text-white" />}
              </button>
            </th>
            <th className="px-3 py-3 bg-brand-50 w-14" />
            {headerCell('vn', 'Вн. номер')}
            {headerCell('name', 'Наименование')}
            {headerCell('category', 'Категория')}
            <th className="px-3 py-3 text-left text-caption text-ink-500 bg-brand-50">Статус</th>
            <th className="px-3 py-3 text-left text-caption text-ink-500 bg-brand-50">Объект</th>
            <th className="px-3 py-3 text-left text-caption text-ink-500 bg-brand-50">Склад</th>
            {headerCell('qty', 'Кол-во')}
            <th className="w-12 px-3 py-3 bg-brand-50" />
          </tr>
        </thead>
        <tbody>
          {sorted.map((item, i) => {
            const photo = titlePhoto(item)
            const checked = selectedIds.has(item.id)
            return (
              <motion.tr
                key={item.id}
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ duration: 0.2, delay: Math.min(i, 12) * 0.02 }}
                onClick={() => navigate(`/tool/${item.id}`)}
                className={cn(
                  'h-14 border-b border-brand-100/70 cursor-pointer transition-colors duration-150 hover:bg-brand-50',
                  checked && 'bg-brand-50/70'
                )}
              >
                <td className="px-3" onClick={(e) => e.stopPropagation()}>
                  <button
                    onClick={() => onSelectRow(item.id, !checked)}
                    className={cn(
                      'w-5 h-5 rounded-md border-[1.5px] flex items-center justify-center transition-colors',
                      checked ? 'bg-brand-600 border-brand-600' : 'border-brand-100 bg-white',
                      !checked && !selectionMode && 'lg:opacity-40'
                    )}
                  >
                    {checked && <Check size={13} className="text-white" />}
                  </button>
                </td>
                <td className="px-3">
                  {photo ? (
                    <img
                      src={photo.url}
                      alt=""
                      className="w-10 h-10 rounded-lg object-cover border border-brand-100/60"
                      loading="lazy"
                    />
                  ) : (
                    <span className="w-10 h-10 rounded-lg bg-brand-50 border border-brand-100/60 flex items-center justify-center text-brand-100">
                      <Wrench size={16} strokeWidth={1.5} />
                    </span>
                  )}
                </td>
                <td className="px-3 font-mono-num text-ink-500 whitespace-nowrap">
                  <Highlight text={item.internalId} query={query} />
                </td>
                <td className="px-3 text-sm font-semibold text-ink-900 max-w-[260px]">
                  <span className="line-clamp-1">
                    <Highlight text={item.title} query={query} />
                  </span>
                </td>
                <td className="px-3 text-[13px] text-ink-500 whitespace-nowrap">{item.category?.name ?? '—'}</td>
                <td className="px-3">
                  <ItemStatusBadge item={item} />
                </td>
                <td className="px-3 text-[13px] text-ink-500 whitespace-nowrap">{item.buildingSite?.name ?? '—'}</td>
                <td className="px-3 text-[13px] text-ink-500 whitespace-nowrap">{item.storage?.name ?? '—'}</td>
                <td className="px-3 font-mono-num text-ink-900">
                  {item.quantitative && typeof item.quantity === 'number' ? `${item.quantity} ${item.unit ?? 'шт'}` : ''}
                </td>
                <td className="px-3 relative" onClick={(e) => e.stopPropagation()}>
                  <button
                    onClick={() => setMenuFor(menuFor === item.id ? null : item.id)}
                    className="w-8 h-8 flex items-center justify-center rounded-lg text-ink-300 hover:bg-brand-50 hover:text-ink-900 transition-colors"
                  >
                    <MoreVertical size={16} />
                  </button>
                  {menuFor === item.id && (
                    <>
                      <div className="fixed inset-0 z-40" onClick={() => setMenuFor(null)} />
                      <div className="absolute right-3 top-full mt-1 w-44 rounded-xl border border-brand-100 bg-surface p-1 shadow-hover z-50">
                        <button
                          onClick={() => navigate(`/tool/${item.id}`)}
                          className="w-full text-left rounded-lg px-3 py-2 text-sm font-semibold text-ink-900 hover:bg-brand-50 transition-colors"
                        >
                          Открыть
                        </button>
                        <button
                          onClick={() => navigate('/transfers')}
                          className="w-full text-left rounded-lg px-3 py-2 text-sm font-semibold text-ink-900 hover:bg-brand-50 transition-colors"
                        >
                          Передать
                        </button>
                        <button
                          onClick={() => setMenuFor(null)}
                          className="w-full text-left rounded-lg px-3 py-2 text-sm font-semibold text-ink-900 hover:bg-brand-50 transition-colors"
                        >
                          Печать QR
                        </button>
                        <button
                          onClick={() => {
                            onDelete(item)
                            setMenuFor(null)
                          }}
                          className="w-full text-left rounded-lg px-3 py-2 text-sm font-semibold text-danger hover:bg-danger-bg transition-colors"
                        >
                          Удалить
                        </button>
                      </div>
                    </>
                  )}
                </td>
              </motion.tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}

// ─── Главный компонент ───────────────────────────────────────────────────────

export default function MyTools() {
  const navigate = useNavigate()
  const utils = trpc.useUtils()
  const [renderedAt] = useState(Date.now)

  // Данные (tRPC)
  const meQuery = trpc.meta.currentUser.useQuery()
  const countsQuery = trpc.meta.transferCounts.useQuery()
  const incomingQuery = trpc.transfers.incoming.useQuery()
  const mineAllQuery = trpc.items.list.useQuery({ onlyMine: true, page: 1, limit: FETCH_LIMIT })
  const categoriesQuery = trpc.admin.dictionaries.list.useQuery({ kind: 'categories' })
  const brandsQuery = trpc.admin.dictionaries.list.useQuery({ kind: 'brands' })
  const statusesQuery = trpc.admin.dictionaries.list.useQuery({ kind: 'statuses' })
  const storagesQuery = trpc.admin.storages.list.useQuery({})
  const sitesQuery = trpc.admin.buildingSites.list.useQuery({})

  const [view, setView] = useState<ViewMode>('grid')
  const [sort, setSort] = useState<SortKey>('createdAt_desc')
  const [query, setQuery] = useState('')
  const [debouncedQuery, setDebouncedQuery] = useState('')
  const [applied, setApplied] = useState<Filters>(emptyFilters)
  const [pending, setPending] = useState<Filters>(emptyFilters)
  const [filtersOpen, setFiltersOpen] = useState(false)
  const [activeTab, setActiveTab] = useState(0)
  const [visible, setVisible] = useState(PAGE_SIZE)
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set())
  const [acceptTransfer, setAcceptTransfer] = useState<Transfer | null>(null)
  const [toast, setToast] = useState<string | null>(null)
  const sentinelRef = useRef<HTMLDivElement>(null)
  const searchRef = useRef<HTMLInputElement>(null)

  // Поиск (debounce 300ms) — серверный
  useEffect(() => {
    const t = setTimeout(() => setDebouncedQuery(query), 300)
    return () => clearTimeout(t)
  }, [query])

  const listQuery = trpc.items.list.useQuery({
    onlyMine: true,
    search: debouncedQuery.trim() || undefined,
    sort,
    page: 1,
    limit: FETCH_LIMIT,
  })

  // Мутации с инвалидацией
  const updateMutation = trpc.items.update.useMutation({
    onSuccess: () => {
      utils.items.list.invalidate()
      utils.meta.transferCounts.invalidate()
    },
  })
  const removeMutation = trpc.items.remove.useMutation({
    onSuccess: () => {
      utils.items.list.invalidate()
      utils.meta.transferCounts.invalidate()
      setToast('Инструмент удалён')
    },
    onError: (e) => setToast(e.message),
  })

  // Тост автозакрытие
  useEffect(() => {
    if (!toast) return
    const t = setTimeout(() => setToast(null), 4000)
    return () => clearTimeout(t)
  }, [toast])

  const me = meQuery.data
  const mineAll = useMemo(() => mineAllQuery.data?.rows ?? [], [mineAllQuery.data])
  const rows = useMemo(() => listQuery.data?.rows ?? [], [listQuery.data])
  const filtered = useMemo(() => applyFilters(rows, applied), [rows, applied])
  const total = filtered.length
  const shown = filtered.slice(0, visible)
  const hasMore = visible < total

  const incomingTransfers = useMemo(() => incomingQuery.data ?? [], [incomingQuery.data])
  const incomingByItem = useMemo(() => {
    const map = new Map<number, Transfer>()
    incomingTransfers.forEach((t) => {
      if (t.itemId) map.set(t.itemId, t)
    })
    return map
  }, [incomingTransfers])

  // Персональная сводка
  const totalMine = mineAll.length
  const incomingCount = countsQuery.data?.incoming ?? incomingTransfers.length
  const outgoingCount = countsQuery.data?.outgoing ?? 0
  const overdueCount = useMemo(() => {
    return mineAll.filter((i) => i.notifyDate && new Date(i.notifyDate).getTime() < renderedAt).length
  }, [mineAll, renderedAt])

  // Сброс видимого диапазона при смене фильтров/поиска/сортировки
  useEffect(() => {
    const frame = requestAnimationFrame(() => setVisible(PAGE_SIZE))
    return () => cancelAnimationFrame(frame)
  }, [applied, debouncedQuery, sort])

  const loadMore = () => {
    if (!hasMore) return
    setVisible((v) => v + PAGE_SIZE)
  }

  // Auto-load при приближении к низу (десктоп)
  useEffect(() => {
    const el = sentinelRef.current
    if (!el) return
    const obs = new IntersectionObserver(
      (entries) => {
        if (entries[0].isIntersecting && window.innerWidth >= 1024) loadMore()
      },
      { rootMargin: '400px' }
    )
    obs.observe(el)
    return () => obs.disconnect()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasMore])

  const selectionMode = selectedIds.size > 0
  const activeFilterCount = countFilters(applied)
  const pendingCount = countFilters(pending)

  const togglePending = (key: keyof Omit<Filters, 'qr'>, id: number) => {
    setPending((prev) => {
      const list = prev[key]
      return { ...prev, [key]: list.includes(id) ? list.filter((x) => x !== id) : [...list, id] }
    })
  }

  const applyFiltersNow = () => {
    setApplied(pending)
    setFiltersOpen(false)
  }

  const resetAll = () => {
    setApplied(emptyFilters)
    setPending(emptyFilters)
    setQuery('')
  }

  const toggleSelect = (id: number) => {
    setSelectedIds((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const setRowSelected = (id: number, checked: boolean) => {
    setSelectedIds((prev) => {
      const next = new Set(prev)
      if (checked) next.add(id)
      else next.delete(id)
      return next
    })
  }

  const onCallClick = (item: Item) => {
    if (item.responsible) setToast(`Звоним: ${item.responsible.fullName}…`)
  }

  const onDelete = (item: Item) => {
    if (window.confirm(`Удалить «${item.internalId} ${item.title}» из каталога?`)) {
      removeMutation.mutate({ id: item.id })
    }
  }

  const bulkSetStatus = (statusId: number) => {
    const status = statusesQuery.data?.find((s) => s.id === statusId)
    const name = status?.name ?? ''
    const ids = [...selectedIds]
    let reason: string | undefined
    if (statusNeedsReason(status?.slug)) {
      const answer = askStatusReason(name)
      if (!answer) {
        setToast('Без причины статус не меняем — нужно минимум 3 символа')
        return
      }
      reason = answer
    }
    Promise.all(ids.map((id) => updateMutation.mutateAsync({ id, statusId, reason })))
      .then(() => {
        setToast(`Статус изменён: ${name} (${ids.length} ед.)`)
        setSelectedIds(new Set())
      })
      .catch((e: Error) => setToast(e.message))
  }

  // Чипы активных фильтров
  const chips: { key: string; label: string; remove: () => void }[] = []
  applied.sites.forEach((id) =>
    chips.push({
      key: `s-${id}`,
      label: sitesQuery.data?.find((s) => s.id === id)?.name ?? String(id),
      remove: () => setApplied((p) => ({ ...p, sites: p.sites.filter((x) => x !== id) })),
    })
  )
  applied.warehouses.forEach((id) =>
    chips.push({
      key: `w-${id}`,
      label: storagesQuery.data?.find((s) => s.id === id)?.name ?? String(id),
      remove: () => setApplied((p) => ({ ...p, warehouses: p.warehouses.filter((x) => x !== id) })),
    })
  )
  applied.categories.forEach((id) =>
    chips.push({
      key: `c-${id}`,
      label: categoriesQuery.data?.find((c) => c.id === id)?.name ?? String(id),
      remove: () => setApplied((p) => ({ ...p, categories: p.categories.filter((x) => x !== id) })),
    })
  )
  applied.brands.forEach((id) =>
    chips.push({
      key: `b-${id}`,
      label: brandsQuery.data?.find((b) => b.id === id)?.name ?? String(id),
      remove: () => setApplied((p) => ({ ...p, brands: p.brands.filter((x) => x !== id) })),
    })
  )
  applied.statuses.forEach((id) =>
    chips.push({
      key: `st-${id}`,
      label: statusesQuery.data?.find((s) => s.id === id)?.name ?? String(id),
      remove: () => setApplied((p) => ({ ...p, statuses: p.statuses.filter((x) => x !== id) })),
    })
  )
  if (applied.qr)
    chips.push({
      key: 'qr',
      label: applied.qr === 'with' ? 'С QR-кодом' : 'Без QR-кода',
      remove: () => setApplied((p) => ({ ...p, qr: null })),
    })

  const countBy = (fn: (t: Item) => boolean) => rows.filter(fn).length

  const filterTabs = [
    { label: 'Объекты', count: pending.sites.length },
    { label: 'Склады', count: pending.warehouses.length },
    { label: 'Категории', count: pending.categories.length },
    { label: 'Бренды', count: pending.brands.length },
    { label: 'Статусы', count: pending.statuses.length },
    { label: 'QR', count: pending.qr ? 1 : 0 },
  ]

  const checkboxGrid = 'grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-1'

  // Содержимое вкладок фильтров
  const tabContent = [
    <div key="t0" className={checkboxGrid}>
      {(sitesQuery.data ?? []).map((s) => (
        <FilterCheckbox
          key={s.id}
          checked={pending.sites.includes(s.id)}
          onChange={() => togglePending('sites', s.id)}
          label={s.name}
          count={countBy((t) => t.buildingSiteId === s.id)}
        />
      ))}
    </div>,
    <div key="t1" className={checkboxGrid}>
      {(storagesQuery.data ?? []).map((w) => (
        <FilterCheckbox
          key={w.id}
          checked={pending.warehouses.includes(w.id)}
          onChange={() => togglePending('warehouses', w.id)}
          label={w.name}
          count={countBy((t) => t.storageId === w.id)}
        />
      ))}
    </div>,
    <div key="t2" className={checkboxGrid}>
      {(categoriesQuery.data ?? []).map((c) => (
        <FilterCheckbox
          key={c.id}
          checked={pending.categories.includes(c.id)}
          onChange={() => togglePending('categories', c.id)}
          label={c.name}
          count={countBy((t) => t.categoryId === c.id)}
        />
      ))}
    </div>,
    <div key="t3" className={checkboxGrid}>
      {(brandsQuery.data ?? []).map((b) => (
        <FilterCheckbox
          key={b.id}
          checked={pending.brands.includes(b.id)}
          onChange={() => togglePending('brands', b.id)}
          label={b.name}
          count={countBy((t) => t.brandId === b.id)}
        />
      ))}
    </div>,
    <div key="t4" className={checkboxGrid}>
      {(statusesQuery.data ?? []).map((s) => (
        <FilterCheckbox
          key={s.id}
          checked={pending.statuses.includes(s.id)}
          onChange={() => togglePending('statuses', s.id)}
          label={s.name}
          dot={s.color ?? undefined}
          count={countBy((t) => t.statusId === s.id)}
        />
      ))}
    </div>,
    <div key="t5" className="flex flex-wrap gap-2">
      {([
        { v: 'with' as const, label: 'С QR-кодом', count: countBy((t) => !!t.qrCode) },
        { v: 'without' as const, label: 'Без QR-кода', count: countBy((t) => !t.qrCode) },
      ]).map((opt) => (
        <button
          key={opt.v}
          onClick={() => setPending((p) => ({ ...p, qr: p.qr === opt.v ? null : opt.v }))}
          className={cn(
            'flex items-center gap-2 rounded-full border px-3.5 py-2 text-sm font-semibold transition-colors',
            pending.qr === opt.v
              ? 'border-brand-600 bg-brand-50 text-brand-600'
              : 'border-brand-100 text-ink-500 hover:bg-brand-50/60'
          )}
        >
          <span
            className={cn(
              'w-4 h-4 rounded-full border-[1.5px] flex items-center justify-center',
              pending.qr === opt.v ? 'border-brand-600' : 'border-brand-100'
            )}
          >
            {pending.qr === opt.v && <span className="w-2 h-2 rounded-full bg-brand-600" />}
          </span>
          {opt.label}
          <span className="font-mono-num text-ink-300">({opt.count})</span>
        </button>
      ))}
    </div>,
  ]

  // Панель фильтров (общая для accordion и drawer)
  const filterPanel = (
    <div>
      <div className="flex gap-1 overflow-x-auto border-b border-brand-100/70 -mx-1 px-1">
        {filterTabs.map((tab, i) => (
          <button
            key={tab.label}
            onClick={() => setActiveTab(i)}
            className={cn(
              'relative shrink-0 px-3 py-2.5 text-sm font-semibold transition-colors',
              activeTab === i ? 'text-brand-600' : 'text-ink-500 hover:text-ink-900'
            )}
          >
            {tab.label}
            {tab.count > 0 && (
              <span className={cn('ml-1 font-mono-num', activeTab === i ? 'text-accent' : 'text-ink-300')}>
                ({tab.count})
              </span>
            )}
            {activeTab === i && (
              <motion.span
                layoutId="my-filter-tab-underline"
                className="absolute bottom-0 left-2 right-2 h-0.5 rounded-full bg-accent"
                transition={{ duration: 0.2 }}
              />
            )}
          </button>
        ))}
      </div>
      <AnimatePresence mode="wait">
        <motion.div
          key={activeTab}
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -4 }}
          transition={{ duration: 0.18 }}
          className="py-4 min-h-[120px]"
        >
          {tabContent[activeTab]}
        </motion.div>
      </AnimatePresence>
      <div className="flex items-center justify-between gap-3 pt-3 border-t border-brand-100/70">
        <span className="text-sm text-ink-500">
          Выбрано: <span className="font-mono-num text-ink-900">{pendingCount}</span> параметров
        </span>
        <div className="flex gap-2">
          <button
            onClick={() => setPending(emptyFilters)}
            className="h-10 px-4 rounded-xl text-sm font-semibold text-brand-600 hover:bg-brand-50 transition-colors"
          >
            Сбросить
          </button>
          <button
            onClick={applyFiltersNow}
            className="h-10 px-5 rounded-xl bg-accent text-white text-sm font-semibold hover:bg-accent-hover hover:-translate-y-px active:scale-[0.97] transition"
          >
            Применить
          </button>
        </div>
      </div>
    </div>
  )

  // Полоса сводки
  const summary = [
    {
      key: 'total',
      label: 'Всего на мне',
      value: totalMine,
      sub: 'единиц',
      icon: Package,
      valueClass: 'text-ink-900',
      onClick: undefined as (() => void) | undefined,
      pulse: false,
    },
    {
      key: 'incoming',
      label: 'Ожидают моей приёмки',
      value: incomingCount,
      sub: 'передачи ко мне',
      icon: Inbox,
      valueClass: 'text-warning',
      onClick: () => navigate('/transfers?tab=receive'),
      pulse: incomingCount > 0,
    },
    {
      key: 'outgoing',
      label: 'Я передал, ждут приёмки',
      value: outgoingCount,
      sub: 'в процессе',
      icon: Send,
      valueClass: 'text-brand-600',
      onClick: () => navigate('/transfers'),
      pulse: false,
    },
    {
      key: 'overdue',
      label: 'Просроченные возвраты',
      value: overdueCount,
      sub: 'требуют внимания',
      icon: AlarmClock,
      valueClass: 'text-danger',
      onClick: () => navigate('/history'),
      pulse: false,
    },
  ]

  const isInitialLoading = listQuery.isLoading || mineAllQuery.isLoading
  const isError = listQuery.isError || mineAllQuery.isError
  const nothingMine = !isInitialLoading && !isError && mineAll.length === 0
  const nothingFound = !nothingMine && total === 0

  return (
    <div className="space-y-4">
      {/* ── Секция 1. Заголовок ── */}
      <div className="flex flex-wrap items-center gap-3">
        <motion.div
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.28, ease: [0.22, 1, 0.36, 1] }}
        >
          <h1 className="text-2xl lg:text-[28px] leading-9 font-bold tracking-[-0.01em] text-ink-900">
            Мои ТМЦ{' '}
            <span className="font-mono-num text-ink-500 font-semibold">({totalMine} ед.)</span>
          </h1>
          <p className="text-[13px] text-ink-500 mt-0.5">на моём ответственном хранении</p>
        </motion.div>
        <div className="flex-1" />
        <motion.div
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.28, delay: 0.05, ease: [0.22, 1, 0.36, 1] }}
        >
          <Link
            to="/create"
            className="inline-flex items-center gap-2 h-10 px-5 rounded-xl bg-accent text-white text-sm font-semibold hover:bg-accent-hover hover:-translate-y-px active:scale-[0.97] transition"
          >
            <Plus size={16} strokeWidth={2.25} />
            Создать инструмент
          </Link>
        </motion.div>
      </div>

      {/* ── Секция 2. Полоса сводки ── */}
      <div className="flex gap-3 overflow-x-auto snap-x snap-mandatory pb-1 -mx-1 px-1 sm:grid sm:grid-cols-2 sm:overflow-visible xl:grid-cols-4">
        {summary.map((card, i) => (
          <motion.button
            key={card.key}
            onClick={card.onClick}
            disabled={!card.onClick}
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.28, delay: 0.06 * i, ease: [0.22, 1, 0.36, 1] }}
            whileHover={card.onClick ? { y: -2 } : undefined}
            className={cn(
              'min-w-[200px] snap-start text-left bg-surface rounded-card border border-brand-100/60 shadow-card px-5 py-4 flex items-center gap-3 transition-shadow',
              card.onClick && 'hover:shadow-hover cursor-pointer'
            )}
          >
            <span className="relative w-10 h-10 shrink-0 rounded-full bg-brand-50 flex items-center justify-center text-brand-600">
              <card.icon size={18} strokeWidth={1.75} />
              {card.pulse && (
                <motion.span
                  animate={{ scale: [1, 1.5, 1], opacity: [0.7, 0, 0.7] }}
                  transition={{ duration: 1.6, repeat: Infinity, ease: 'easeInOut' }}
                  className="absolute -top-0.5 -right-0.5 w-2.5 h-2.5 rounded-full bg-accent"
                />
              )}
            </span>
            <span className="min-w-0">
              <span className={cn('flex items-baseline gap-2 text-[32px] leading-9 font-bold', card.valueClass)}>
                <CountUp value={card.value} />
              </span>
              <span className="block text-[13px] leading-[18px] text-ink-500 truncate">
                {card.label} · {card.sub}
              </span>
            </span>
          </motion.button>
        ))}
      </div>

      {/* ── Секция 3. Чипы (pinned ответственный + активные фильтры) ── */}
      <div className="flex flex-wrap items-center gap-2">
        <motion.span
          initial={{ opacity: 0, scale: 0.8, boxShadow: '0 0 0 0 rgba(94,98,155,0.5)' }}
          animate={{ opacity: 1, scale: 1, boxShadow: '0 0 0 6px rgba(94,98,155,0)' }}
          transition={{ duration: 0.9, delay: 0.15 }}
          className="inline-flex items-center gap-1.5 rounded-full bg-brand-50 border border-brand-100/70 px-3 py-1 text-[13px] font-semibold text-ink-900"
        >
          <Pin size={12} className="text-brand-600" />
          Ответственный: {me?.fullName ?? '…'}
        </motion.span>
        <AnimatePresence>
          {chips.map((chip) => (
            <motion.span
              key={chip.key}
              initial={{ opacity: 0, scale: 0.8 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.8 }}
              transition={{ type: 'spring', stiffness: 400, damping: 26 }}
              className="inline-flex items-center gap-1.5 rounded-full bg-brand-50 border border-brand-100/70 px-3 py-1 text-[13px] font-semibold text-ink-900"
            >
              {chip.label}
              <button onClick={chip.remove} className="text-ink-300 hover:text-accent transition-colors">
                <X size={13} />
              </button>
            </motion.span>
          ))}
        </AnimatePresence>
        {chips.length > 0 && (
          <button
            onClick={resetAll}
            className="text-[13px] font-semibold text-brand-600 hover:bg-brand-50 rounded-full px-3 py-1 transition-colors"
          >
            Сбросить всё
          </button>
        )}
      </div>

      {/* ── Панель массовых действий / toolbar ── */}
      <AnimatePresence mode="wait">
        {selectedIds.size > 0 ? (
          <motion.div
            key="bulk"
            initial={{ opacity: 0, y: -12 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -12 }}
            transition={{ duration: 0.2 }}
            className="bg-surface rounded-card border border-brand-100/60 shadow-card px-4 py-3 flex flex-wrap items-center gap-2 sm:gap-3"
          >
            <span className="text-sm font-semibold text-ink-900">
              Выбрано: <span className="font-mono-num text-accent">{selectedIds.size}</span>
            </span>
            <button
              onClick={() => setSelectedIds(new Set())}
              className="text-sm font-semibold text-brand-600 hover:bg-brand-50 rounded-lg px-2 py-1 transition-colors"
            >
              Снять выбор
            </button>
            <div className="flex-1" />
            <button
              onClick={() => navigate('/transfers')}
              className="inline-flex items-center gap-1.5 h-8 px-3.5 rounded-xl bg-accent text-white text-[13px] font-semibold hover:bg-accent-hover active:scale-[0.97] transition"
            >
              <ArrowLeftRight size={14} />
              Передать
            </button>
            <select
              defaultValue=""
              disabled={updateMutation.isPending}
              onChange={(e) => {
                if (e.target.value) bulkSetStatus(Number(e.target.value))
                e.target.value = ''
              }}
              className="h-8 rounded-xl border border-brand-100 bg-white px-2.5 text-[13px] font-semibold text-ink-900 disabled:opacity-60"
            >
              <option value="" disabled>
                Изменить статус
              </option>
              {(statusesQuery.data ?? []).map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
            <button
              onClick={() => setToast('QR-коды отправлены на печать')}
              className="inline-flex items-center gap-1.5 h-8 px-3.5 rounded-xl border border-brand-100 bg-white text-[13px] font-semibold text-ink-900 hover:bg-brand-50 transition"
            >
              <QrCode size={14} />
              Печать QR
            </button>
            <button
              onClick={() => setToast('Списание доступно руководителю')}
              className="inline-flex items-center gap-1.5 h-8 px-3.5 rounded-xl border border-danger text-[13px] font-semibold text-danger hover:bg-danger-bg transition"
            >
              <Trash2 size={14} />
              Списать
            </button>
          </motion.div>
        ) : (
          <motion.div
            key="toolbar"
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 12 }}
            transition={{ duration: 0.24, delay: 0.1 }}
            className="bg-surface rounded-card border border-brand-100/60 shadow-card px-3 sm:px-4 py-3 flex flex-wrap items-center gap-2"
          >
            {/* Поиск */}
            <div className="relative flex-1 min-w-[200px] max-w-[420px]">
              <Search size={17} className="absolute left-3 top-1/2 -translate-y-1/2 text-ink-300" />
              <input
                ref={searchRef}
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Название или вн. номер…"
                className="h-10 w-full rounded-xl border border-brand-100 bg-white pl-9 pr-8 text-sm text-ink-900 placeholder:text-ink-300 focus:border-brand-600 focus:shadow-[0_0_0_3px_#5E629B22] transition-shadow"
              />
              {query && (
                <button
                  onClick={() => setQuery('')}
                  className="absolute right-2.5 top-1/2 -translate-y-1/2 text-ink-300 hover:text-ink-500"
                >
                  <X size={15} />
                </button>
              )}
            </div>

            {/* Фильтры */}
            <button
              onClick={() => {
                setPending(applied)
                setFiltersOpen((v) => !v)
              }}
              className={cn(
                'relative inline-flex items-center gap-2 h-10 px-4 rounded-xl border text-sm font-semibold transition-colors',
                filtersOpen || activeFilterCount > 0
                  ? 'border-brand-600 text-brand-600 bg-brand-50'
                  : 'border-brand-100 text-ink-900 bg-white hover:bg-brand-50'
              )}
            >
              <SlidersHorizontal size={16} strokeWidth={1.75} />
              Фильтры
              {activeFilterCount > 0 && (
                <span className="min-w-5 h-5 px-1 rounded-full bg-accent text-white text-[11px] leading-5 text-center font-mono font-semibold animate-badge-pop">
                  {activeFilterCount}
                </span>
              )}
            </button>

            <div className="flex-1" />

            {/* Переключатель вида */}
            <div className="flex rounded-xl border border-brand-100 bg-white p-1">
              {([
                { v: 'grid' as ViewMode, icon: LayoutGrid, label: 'Плитка' },
                { v: 'table' as ViewMode, icon: List, label: 'Таблица' },
              ]).map((opt) => (
                <button
                  key={opt.v}
                  onClick={() => setView(opt.v)}
                  title={opt.label}
                  className="relative w-9 h-8 flex items-center justify-center rounded-lg"
                >
                  {view === opt.v && (
                    <motion.span
                      layoutId="my-view-toggle-pill"
                      className="absolute inset-0 rounded-lg bg-brand-600"
                      transition={{ duration: 0.2 }}
                    />
                  )}
                  <opt.icon
                    size={17}
                    strokeWidth={1.75}
                    className={cn('relative z-10 transition-colors', view === opt.v ? 'text-white' : 'text-ink-300')}
                  />
                </button>
              ))}
            </div>

            {/* Сортировка */}
            <div className="relative">
              <select
                value={sort}
                onChange={(e) => setSort(e.target.value as SortKey)}
                className="appearance-none h-10 rounded-xl border border-brand-100 bg-white pl-3.5 pr-9 text-sm font-semibold text-ink-900 hover:bg-brand-50 cursor-pointer"
              >
                {sortOptions.map((o) => (
                  <option key={o.key} value={o.key}>
                    {o.label}
                  </option>
                ))}
              </select>
              <ArrowUpDown size={14} className="absolute right-3 top-1/2 -translate-y-1/2 text-ink-300 pointer-events-none" />
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── Панель фильтров (десктоп — accordion) ── */}
      <AnimatePresence>
        {filtersOpen && (
          <motion.div
            key="filters"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.26 }}
            className="hidden lg:block overflow-hidden"
          >
            <div className="bg-surface rounded-card border border-brand-100/60 shadow-card px-5 py-3">{filterPanel}</div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Панель фильтров (мобайл — bottom drawer) */}
      <AnimatePresence>
        {filtersOpen && (
          <>
            <motion.div
              key="filters-overlay"
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              transition={{ duration: 0.16 }}
              onClick={() => setFiltersOpen(false)}
              className="lg:hidden fixed inset-0 z-50 bg-[rgba(48,52,102,.45)] backdrop-blur-sm"
            />
            <motion.div
              key="filters-drawer"
              initial={{ y: '100%' }}
              animate={{ y: 0 }}
              exit={{ y: '100%' }}
              transition={{ type: 'spring', stiffness: 300, damping: 30 }}
              className="lg:hidden fixed bottom-0 inset-x-0 z-50 max-h-[85dvh] overflow-y-auto bg-surface rounded-t-modal px-4 pt-2 pb-6 shadow-modal"
            >
              <div className="mx-auto w-10 h-1 rounded-full bg-brand-100 mb-3" />
              {filterPanel}
            </motion.div>
          </>
        )}
      </AnimatePresence>

      {/* ── Секция 4. Список ── */}
      <AnimatePresence mode="wait">
        {isInitialLoading ? (
          <motion.div key="loading" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}>
            <div className="grid grid-cols-2 gap-3 sm:gap-4 [grid-template-columns:repeat(auto-fill,minmax(150px,1fr))] sm:[grid-template-columns:repeat(auto-fill,minmax(240px,1fr))]">
              {Array.from({ length: 8 }).map((_, i) => (
                <CardSkeleton key={`sk-${i}`} />
              ))}
            </div>
          </motion.div>
        ) : isError ? (
          <motion.div
            key="error"
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            className="bg-surface rounded-card border border-brand-100/60 shadow-card py-12 px-6 flex flex-col items-center text-center"
          >
            <h3 className="text-[17px] font-semibold text-ink-900">Не удалось загрузить данные</h3>
            <p className="mt-1 text-[13px] text-ink-500">{listQuery.error?.message ?? 'Проверьте подключение к серверу'}</p>
            <button
              onClick={() => {
                listQuery.refetch()
                mineAllQuery.refetch()
              }}
              className="mt-4 h-10 px-5 rounded-xl bg-accent text-white text-sm font-semibold hover:bg-accent-hover transition"
            >
              Повторить
            </button>
          </motion.div>
        ) : nothingMine ? (
          /* Empty state: на вас ничего не числится */
          <motion.div
            key="empty-mine"
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            className="bg-surface rounded-card border border-brand-100/60 shadow-card py-12 px-6 flex flex-col items-center text-center"
          >
            <img src="/empty-catalog.svg" alt="" className="w-60 h-auto rounded-2xl" />
            <h3 className="mt-5 text-[17px] font-semibold text-ink-900">На вас ничего не числится</h3>
            <p className="mt-1 text-[13px] text-ink-500">Когда вам передадут инструмент, он появится здесь</p>
            <Link
              to="/create"
              className="mt-4 inline-flex items-center gap-2 h-10 px-5 rounded-xl bg-accent text-white text-sm font-semibold hover:bg-accent-hover hover:-translate-y-px active:scale-[0.97] transition"
            >
              <Plus size={16} strokeWidth={2.25} />
              Создать инструмент
            </Link>
          </motion.div>
        ) : nothingFound ? (
          /* Empty state: ничего не найдено */
          <motion.div
            key="empty"
            initial={{ opacity: 0, y: 16 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            className="bg-surface rounded-card border border-brand-100/60 shadow-card py-12 px-6 flex flex-col items-center text-center"
          >
            <img src="/empty-catalog.svg" alt="" className="w-60 h-auto rounded-2xl" />
            <h3 className="mt-5 text-[17px] font-semibold text-ink-900">Ничего не найдено</h3>
            <p className="mt-1 text-[13px] text-ink-500">Попробуйте изменить фильтры или запрос</p>
            <button
              onClick={resetAll}
              className="mt-4 h-10 px-5 rounded-xl text-sm font-semibold text-brand-600 hover:bg-brand-50 transition-colors"
            >
              Сбросить фильтры
            </button>
          </motion.div>
        ) : view === 'grid' ? (
          <motion.div
            key="grid"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
          >
            <div className="grid grid-cols-2 gap-3 sm:gap-4 [grid-template-columns:repeat(auto-fill,minmax(150px,1fr))] sm:[grid-template-columns:repeat(auto-fill,minmax(240px,1fr))]">
              {shown.map((item, i) => (
                <motion.div
                  key={item.id}
                  initial={{ opacity: 0, y: 24 }}
                  whileInView={{ opacity: 1, y: 0 }}
                  viewport={{ once: true, margin: '-40px' }}
                  transition={{ duration: 0.3, delay: (i % 12) * 0.04, ease: [0.22, 1, 0.36, 1] }}
                >
                  <MyToolCard
                    item={item}
                    query={debouncedQuery}
                    selected={selectedIds.has(item.id)}
                    selectionMode={selectionMode}
                    incoming={incomingByItem.get(item.id)}
                    onToggleSelect={() => toggleSelect(item.id)}
                    onAcceptClick={(t) => setAcceptTransfer(t)}
                    onCallClick={onCallClick}
                  />
                </motion.div>
              ))}
            </div>
          </motion.div>
        ) : (
          <motion.div
            key="table"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
            className="bg-surface rounded-card border border-brand-100/60 shadow-card overflow-hidden"
          >
            <MyToolsTable
              items={shown}
              query={debouncedQuery}
              selectedIds={selectedIds}
              selectionMode={selectionMode}
              onSelectRow={setRowSelected}
              onDelete={onDelete}
            />
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── Подгрузка ── */}
      {total > 0 && (
        <div className="flex flex-col items-center gap-3 pt-2 pb-4">
          <span className="text-[13px] text-ink-500">
            Показано <span className="font-mono-num text-ink-900">{Math.min(visible, total)}</span> из{' '}
            <span className="font-mono-num text-ink-900">{total}</span>
          </span>
          {hasMore && (
            <button
              onClick={loadMore}
              className="inline-flex items-center gap-2 h-10 px-6 rounded-xl border border-brand-100 bg-white text-sm font-semibold text-ink-900 hover:bg-brand-50 transition"
            >
              Загрузить ещё
            </button>
          )}
          <div ref={sentinelRef} className="h-px w-full" />
        </div>
      )}

      {/* Модалка приёмки */}
      <AcceptModal transfer={acceptTransfer} onClose={() => setAcceptTransfer(null)} onToast={setToast} />

      {/* Тост */}
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
