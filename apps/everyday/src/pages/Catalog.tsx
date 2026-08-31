import { useEffect, useMemo, useRef, useState } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router'
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
  PackageCheck,
} from 'lucide-react'
import { AnimatePresence, motion } from 'framer-motion'
import { cn } from '@/lib/utils'
import { trpc } from '@/providers/trpc'
import { parseDueInput } from '@/lib/due-date'
import { mapItemToCatalogTool, type CatalogTool } from '@/lib/catalog-item'
import { useStore } from '@/lib/store'
import ToolMiniCard from '@/components/ToolMiniCard'

interface Filters {
  assignees: number[]
  sites: number[]
  warehouses: number[]
  categories: number[]
  brands: number[]
  statuses: number[]
  qr: 'with' | 'without' | null
}

const emptyFilters: Filters = {
  assignees: [],
  sites: [],
  warehouses: [],
  categories: [],
  brands: [],
  statuses: [],
  qr: null,
}

const countFilters = (f: Filters) =>
  f.assignees.length + f.sites.length + f.warehouses.length + f.categories.length + f.brands.length + f.statuses.length + (f.qr ? 1 : 0)

type SortKey = 'new' | 'name' | 'vn' | 'price'
type ViewMode = 'grid' | 'table'

const PAGE_SIZE = 8
const FETCH_LIMIT = 200

const sortOptions: { key: SortKey; label: string }[] = [
  { key: 'new', label: 'Сначала новые' },
  { key: 'name', label: 'По названию А–Я' },
  { key: 'vn', label: 'По вн. номеру' },
  { key: 'price', label: 'По стоимости ↓' },
]

function applyFilters(list: CatalogTool[], f: Filters, q: string): CatalogTool[] {
  const query = q.trim().toLowerCase()
  return list.filter((t) => {
    if (query && !t.name.toLowerCase().includes(query) && !t.vn.toLowerCase().includes(query)) return false
    if (f.assignees.length && !(t.assigneeId && f.assignees.includes(t.assigneeId))) return false
    if (f.sites.length && !(t.siteId && f.sites.includes(t.siteId))) return false
    if (f.warehouses.length && !(t.warehouseId && f.warehouses.includes(t.warehouseId))) return false
    if (f.categories.length && !(t.categoryId && f.categories.includes(t.categoryId))) return false
    if (f.brands.length && !(t.brandId && f.brands.includes(t.brandId))) return false
    if (f.statuses.length && !(t.statusId && f.statuses.includes(t.statusId))) return false
    if (f.qr === 'with' && !t.hasQr) return false
    if (f.qr === 'without' && t.hasQr) return false
    return true
  })
}

function sortTools(list: CatalogTool[], sort: SortKey): CatalogTool[] {
  const arr = [...list]
  switch (sort) {
    case 'new':
      return arr.sort((a, b) => b.createdAt.localeCompare(a.createdAt))
    case 'name':
      return arr.sort((a, b) => a.name.localeCompare(b.name, 'ru'))
    case 'vn':
      return arr.sort((a, b) => a.vn.localeCompare(b.vn, 'ru'))
    case 'price':
      return arr.sort((a, b) => b.price - a.price)
  }
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

// ─── Чекбокс фильтра ─────────────────────────────────────────────────────────

function FilterCheckbox({
  checked,
  onChange,
  label,
  count,
  avatar,
  dot,
}: {
  checked: boolean
  onChange: () => void
  label: string
  count: number
  avatar?: string
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
          <motion.svg
            initial={{ pathLength: 0, opacity: 0 }}
            animate={{ pathLength: 1, opacity: 1 }}
            width="12"
            height="12"
            viewBox="0 0 12 12"
            fill="none"
          >
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
      {avatar && <img src={avatar} alt="" className="w-5 h-5 rounded-full object-cover border border-brand-100 shrink-0" />}
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

// ─── Главный компонент ───────────────────────────────────────────────────────

export default function Catalog() {
  const [searchParams] = useSearchParams()
  const navigate = useNavigate()
  const {
    selectedToolIds,
    setToolSelected,
    clearSelection,
    selectionMode,
    setSelectionMode,
    workspace,
  } = useStore()

  const utils = trpc.useUtils()
  const listQ = trpc.items.list.useQuery(
    { page: 1, limit: FETCH_LIMIT, sort: 'createdAt_desc', workspaceId: workspace?.id },
    { enabled: Boolean(workspace?.id) },
  )
  const usersQ = trpc.admin.users.list.useQuery({})
  const sitesQ = trpc.admin.buildingSites.list.useQuery({})
  const storagesQ = trpc.admin.storages.list.useQuery({})
  const categoriesQ = trpc.admin.dictionaries.list.useQuery({ kind: 'categories' })
  const brandsQ = trpc.admin.dictionaries.list.useQuery({ kind: 'brands' })
  const statusesQ = trpc.admin.dictionaries.list.useQuery({ kind: 'statuses' })

  const tools = useMemo(
    () => (listQ.data?.rows ?? []).map(mapItemToCatalogTool),
    [listQ.data],
  )
  const users = usersQ.data ?? []
  const sites = sitesQ.data ?? []
  const warehouses = storagesQ.data ?? []
  const categories = categoriesQ.data ?? []
  const brands = brandsQ.data ?? []
  const statuses = statusesQ.data ?? []

  const [view, setView] = useState<ViewMode>('grid')
  const [sort, setSort] = useState<SortKey>('new')
  const [query, setQuery] = useState(searchParams.get('q') ?? '')
  const [debouncedQuery, setDebouncedQuery] = useState(query)
  const [applied, setApplied] = useState<Filters>(emptyFilters)
  const [pending, setPending] = useState<Filters>(emptyFilters)
  const [filtersOpen, setFiltersOpen] = useState(false)
  const [activeTab, setActiveTab] = useState(0)
  const [visible, setVisible] = useState(PAGE_SIZE)
  const [loading, setLoading] = useState(false)
  const [toast, setToast] = useState<string | null>(null)
  const takeMany = trpc.transfers.takeMany.useMutation({
    onSuccess: (res) => {
      utils.items.list.invalidate()
      utils.meta.transferCounts.invalidate()
      const skipped = res.failed.length
      setToast(
        skipped
          ? `Взято ${res.takenCount} шт., не удалось: ${skipped}`
          : `Взято ${res.takenCount} шт.`,
      )
      clearSelection()
    },
    onError: (e) => setToast(e.message),
  })
  const sentinelRef = useRef<HTMLDivElement>(null)
  const searchRef = useRef<HTMLInputElement>(null)

  // Debounce поиска 300ms
  useEffect(() => {
    const t = setTimeout(() => setDebouncedQuery(query), 300)
    return () => clearTimeout(t)
  }, [query])

  // Фокус в поиск с мобильной шапки (?focus=search)
  useEffect(() => {
    if (searchParams.get('focus') === 'search') searchRef.current?.focus()
  }, [searchParams])

  // Тост автозакрытие
  useEffect(() => {
    if (!toast) return
    const t = setTimeout(() => setToast(null), 4000)
    return () => clearTimeout(t)
  }, [toast])

  const filtered = useMemo(() => sortTools(applyFilters(tools, applied, debouncedQuery), sort), [tools, applied, debouncedQuery, sort])
  const total = filtered.length
  const shown = filtered.slice(0, visible)
  const hasMore = visible < total

  // Сброс видимого диапазона при смене фильтров/поиска/сортировки
  useEffect(() => {
    const frame = requestAnimationFrame(() => setVisible(PAGE_SIZE))
    return () => cancelAnimationFrame(frame)
  }, [applied, debouncedQuery, sort])

  const loadMore = () => {
    if (loading || !hasMore) return
    setLoading(true)
    setTimeout(() => {
      setVisible((v) => v + PAGE_SIZE)
      setLoading(false)
    }, 600)
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
  }, [hasMore, loading])

  const selectedCount = selectedToolIds.size
  const activeFilterCount = countFilters(applied)
  const pendingCount = countFilters(pending)

  const togglePending = (key: keyof Omit<Filters, 'qr'>, id: number) => {
    setPending((prev) => {
      const list = prev[key] as number[]
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

  // Чипы активных фильтров
  const chips: { key: string; label: string; remove: () => void }[] = []
  applied.assignees.forEach((id) => chips.push({ key: `a-${id}`, label: users.find((u) => u.id === id)?.fullName ?? String(id), remove: () => setApplied((p) => ({ ...p, assignees: p.assignees.filter((x) => x !== id) })) }))
  applied.sites.forEach((id) => chips.push({ key: `s-${id}`, label: sites.find((s) => s.id === id)?.name ?? String(id), remove: () => setApplied((p) => ({ ...p, sites: p.sites.filter((x) => x !== id) })) }))
  applied.warehouses.forEach((id) => chips.push({ key: `w-${id}`, label: warehouses.find((w) => w.id === id)?.name ?? String(id), remove: () => setApplied((p) => ({ ...p, warehouses: p.warehouses.filter((x) => x !== id) })) }))
  applied.categories.forEach((id) => chips.push({ key: `c-${id}`, label: categories.find((c) => c.id === id)?.name ?? String(id), remove: () => setApplied((p) => ({ ...p, categories: p.categories.filter((x) => x !== id) })) }))
  applied.brands.forEach((id) => chips.push({ key: `b-${id}`, label: brands.find((b) => b.id === id)?.name ?? String(id), remove: () => setApplied((p) => ({ ...p, brands: p.brands.filter((x) => x !== id) })) }))
  applied.statuses.forEach((id) => chips.push({ key: `st-${id}`, label: statuses.find((s) => s.id === id)?.name ?? String(id), remove: () => setApplied((p) => ({ ...p, statuses: p.statuses.filter((x) => x !== id) })) }))
  if (applied.qr) chips.push({ key: 'qr', label: applied.qr === 'with' ? 'С QR-кодом' : 'Без QR-кода', remove: () => setApplied((p) => ({ ...p, qr: null })) })

  const onCallClick = (tool: CatalogTool) => {
    if (tool.assigneeName) setToast(`Звоним ${tool.assigneeName}…`)
  }

  const countBy = (fn: (t: CatalogTool) => boolean) => tools.filter(fn).length

  const filterTabs = [
    { label: 'Ответственные', count: pending.assignees.length },
    { label: 'Объекты', count: pending.sites.length },
    { label: 'Склады', count: pending.warehouses.length },
    { label: 'Категории', count: pending.categories.length },
    { label: 'Бренды', count: pending.brands.length },
    { label: 'Статусы', count: pending.statuses.length },
    { label: 'QR', count: pending.qr ? 1 : 0 },
  ]

  // Содержимое вкладок фильтров
  const tabContent = [
    // Ответственные
    <div key="t0" className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-1">
      {users.map((u) => (
        <FilterCheckbox
          key={u.id}
          checked={pending.assignees.includes(u.id)}
          onChange={() => togglePending('assignees', u.id)}
          label={u.fullName}
          avatar={u.avatarUrl ?? undefined}
          count={countBy((t) => t.assigneeId === u.id)}
        />
      ))}
    </div>,
    // Объекты
    <div key="t1" className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-1">
      {sites.map((s) => (
        <FilterCheckbox
          key={s.id}
          checked={pending.sites.includes(s.id)}
          onChange={() => togglePending('sites', s.id)}
          label={s.name}
          count={countBy((t) => t.siteId === s.id)}
        />
      ))}
    </div>,
    // Склады
    <div key="t2" className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-1">
      {warehouses.map((w) => (
        <FilterCheckbox
          key={w.id}
          checked={pending.warehouses.includes(w.id)}
          onChange={() => togglePending('warehouses', w.id)}
          label={w.name}
          count={countBy((t) => t.warehouseId === w.id)}
        />
      ))}
    </div>,
    // Категории
    <div key="t3" className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-1">
      {categories.map((c) => (
        <FilterCheckbox
          key={c.id}
          checked={pending.categories.includes(c.id)}
          onChange={() => togglePending('categories', c.id)}
          label={c.name}
          count={countBy((t) => t.categoryId === c.id)}
        />
      ))}
    </div>,
    // Бренды
    <div key="t4" className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-1">
      {brands.map((b) => (
        <FilterCheckbox
          key={b.id}
          checked={pending.brands.includes(b.id)}
          onChange={() => togglePending('brands', b.id)}
          label={b.name}
          count={countBy((t) => t.brandId === b.id)}
        />
      ))}
    </div>,
    // Статусы
    <div key="t5" className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-4 gap-1">
      {statuses.map((s) => (
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
    // QR
    <div key="t6" className="flex flex-wrap gap-2">
      {([
        { v: 'with' as const, label: 'С QR-кодом', count: countBy((t) => t.hasQr) },
        { v: 'without' as const, label: 'Без QR-кода', count: countBy((t) => !t.hasQr) },
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
      {/* Вкладки */}
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
                layoutId="filter-tab-underline"
                className="absolute bottom-0 left-2 right-2 h-0.5 rounded-full bg-accent"
                transition={{ duration: 0.2 }}
              />
            )}
          </button>
        ))}
      </div>
      {/* Контент вкладки */}
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
      {/* Футер */}
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

  const enterSelection = (id: string, checked: boolean) => {
    if (!selectionMode) setSelectionMode(true)
    setToolSelected(id, checked)
  }

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
            Все ТМЦ{' '}
            <span className="font-mono-num text-ink-500 font-semibold">({listQ.data?.total ?? tools.length} ед.)</span>
          </h1>
        </motion.div>
        <div className="flex-1" />
        <motion.div
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.28, delay: 0.05, ease: [0.22, 1, 0.36, 1] }}
          className="flex flex-wrap items-center gap-2"
        >
          <Link
            to="/invite"
            className="inline-flex items-center gap-2 h-10 px-4 rounded-xl bg-accent text-white text-sm font-semibold hover:bg-accent-hover hover:-translate-y-px active:scale-[0.97] transition"
          >
            <QrCode size={16} strokeWidth={2.25} />
            Пригласить по QR
          </Link>
          <button
            onClick={() => navigate('/scan')}
            className="inline-flex items-center gap-2 h-10 px-4 rounded-xl border border-brand-100 bg-white text-sm font-semibold text-ink-900 hover:bg-brand-50 transition"
          >
            <QrCode size={16} strokeWidth={2.25} />
            Сканировать QR
          </button>
          <Link
            to="/create"
            className="inline-flex items-center gap-2 h-10 px-5 rounded-xl border border-brand-100 bg-white text-sm font-semibold text-ink-900 hover:bg-brand-50 transition"
          >
            <Plus size={16} strokeWidth={2.25} />
            Создать инструмент
          </Link>
        </motion.div>
      </div>

      {/* Чипы активных фильтров */}
      <AnimatePresence>
        {chips.length > 0 && (
          <motion.div className="flex flex-wrap items-center gap-2">
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
            <button
              onClick={resetAll}
              className="text-[13px] font-semibold text-brand-600 hover:bg-brand-50 rounded-full px-3 py-1 transition-colors"
            >
              Сбросить всё
            </button>
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── Секция 4. Панель массовых действий (заменяет toolbar) ── */}
      <AnimatePresence mode="wait">
        {selectedCount > 0 ? (
          <motion.div
            key="bulk"
            initial={{ opacity: 0, y: -12 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -12 }}
            transition={{ duration: 0.2 }}
            className="bg-surface rounded-card border border-brand-100/60 shadow-card px-4 py-3 flex flex-wrap items-center gap-2 sm:gap-3"
          >
            <span className="text-sm font-semibold text-ink-900">
              Выбрано: <span className="font-mono-num text-accent">{selectedCount}</span>
            </span>
            <button
              onClick={clearSelection}
              className="text-sm font-semibold text-brand-600 hover:bg-brand-50 rounded-lg px-2 py-1 transition-colors"
            >
              Снять выбор
            </button>
            <div className="flex-1" />
            <button
              onClick={() => {
                const ids = [...selectedToolIds].map(Number).filter((n) => Number.isFinite(n) && n > 0)
                if (!ids.length) return
                const due = window.prompt('Срок возврата (ГГГГ-ММ-ДД ЧЧ:ММ). Пусто = без срока')
                if (due === null) return
                const parsed = parseDueInput(due)
                if (!parsed.ok) {
                  window.alert('Не понял дату. Формат: 2026-09-01 18:00')
                  return
                }
                takeMany.mutate({ itemIds: ids, dueAt: parsed.iso })
              }}
              disabled={takeMany.isPending}
              className="inline-flex items-center gap-1.5 h-8 px-3.5 rounded-xl bg-accent text-white text-[13px] font-semibold hover:bg-accent-hover active:scale-[0.97] transition disabled:opacity-60"
            >
              {takeMany.isPending ? <Loader2 size={14} className="animate-spin" /> : <PackageCheck size={14} />}
              Взять выбранные
            </button>
            <button
              onClick={() => navigate('/transfers')}
              className="inline-flex items-center gap-1.5 h-8 px-3.5 rounded-xl border border-brand-100 bg-white text-[13px] font-semibold text-ink-900 hover:bg-brand-50 transition"
            >
              <ArrowLeftRight size={14} />
              Передать
            </button>
            <select
              defaultValue=""
              onChange={(e) => {
                if (e.target.value) setToast(`Статус: ${statuses.find((s) => String(s.id) === e.target.value)?.name ?? ''}`)
                e.target.value = ''
              }}
              className="h-8 rounded-xl border border-brand-100 bg-white px-2.5 text-[13px] font-semibold text-ink-900"
            >
              <option value="" disabled>
                Изменить статус
              </option>
              {statuses.map((s) => (
                <option key={s.id} value={String(s.id)}>
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
          /* ── Секция 2. Панель инструментов ── */
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
                      layoutId="view-toggle-pill"
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

      {/* ── Секция 3. Панель фильтров (десктоп — accordion) ── */}
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
            <div className="bg-surface rounded-card border border-brand-100/60 shadow-card px-5 py-3">
              {filterPanel}
            </div>
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

      {/* ── Секции 5–6. Контент ── */}
      <AnimatePresence mode="wait">
        {listQ.isLoading ? (
          <motion.div key="loading" className="grid grid-cols-2 gap-3 sm:gap-4 [grid-template-columns:repeat(auto-fill,minmax(150px,1fr))] sm:[grid-template-columns:repeat(auto-fill,minmax(240px,1fr))]">
            {Array.from({ length: 8 }).map((_, i) => (
              <CardSkeleton key={`sk-${i}`} />
            ))}
          </motion.div>
        ) : total === 0 ? (
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
              {shown.map((tool, i) => (
                <motion.div
                  key={tool.id}
                  initial={{ opacity: 0, y: 24 }}
                  whileInView={{ opacity: 1, y: 0 }}
                  viewport={{ once: true, margin: '-40px' }}
                  transition={{ duration: 0.3, delay: (i % 12) * 0.04, ease: [0.22, 1, 0.36, 1] }}
                >
                  <ToolMiniCard tool={tool} selectionMode={selectionMode} onCallClick={onCallClick} />
                </motion.div>
              ))}
              {loading &&
                Array.from({ length: 4 }).map((_, i) => <CardSkeleton key={`sk-${i}`} />)}
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
            <CatalogTable
              tools={shown}
              query={debouncedQuery}
              selectionMode={selectionMode}
              onSelectRow={enterSelection}
            />
          </motion.div>
        )}
      </AnimatePresence>

      {/* ── Секция 7. Подгрузка ── */}
      {total > 0 && (
        <div className="flex flex-col items-center gap-3 pt-2 pb-4">
          <span className="text-[13px] text-ink-500">
            Показано <span className="font-mono-num text-ink-900">{Math.min(visible, total)}</span> из{' '}
            <span className="font-mono-num text-ink-900">{total}</span>
          </span>
          {hasMore && (
            <button
              onClick={loadMore}
              disabled={loading}
              className="inline-flex items-center gap-2 h-10 px-6 rounded-xl border border-brand-100 bg-white text-sm font-semibold text-ink-900 hover:bg-brand-50 disabled:opacity-60 transition"
            >
              {loading && <Loader2 size={16} className="animate-spin" />}
              {loading ? 'Загружаем…' : 'Загрузить ещё'}
            </button>
          )}
          <div ref={sentinelRef} className="h-px w-full" />
        </div>
      )}

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

// ─── Табличный вид (секция 6) ────────────────────────────────────────────────

function CatalogTable({
  tools,
  query,
  selectionMode,
  onSelectRow,
}: {
  tools: CatalogTool[]
  query: string
  selectionMode: boolean
  onSelectRow: (id: string, checked: boolean) => void
}) {
  const navigate = useNavigate()
  const { selectedToolIds } = useStore()
  const [menuFor, setMenuFor] = useState<string | null>(null)
  const [sortCol, setSortCol] = useState<string | null>(null)
  const [sortDir, setSortDir] = useState<'asc' | 'desc'>('asc')

  const sorted = useMemo(() => {
    if (!sortCol) return tools
    const arr = [...tools]
    const dir = sortDir === 'asc' ? 1 : -1
    switch (sortCol) {
      case 'vn':
        return arr.sort((a, b) => a.vn.localeCompare(b.vn, 'ru') * dir)
      case 'name':
        return arr.sort((a, b) => a.name.localeCompare(b.name, 'ru') * dir)
      case 'category':
        return arr.sort((a, b) => a.categoryName.localeCompare(b.categoryName, 'ru') * dir)
      case 'qty':
        return arr.sort((a, b) => ((a.quantity ?? 0) - (b.quantity ?? 0)) * dir)
      default:
        return arr
    }
  }, [tools, sortCol, sortDir])

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
            <ArrowDown size={12} className="opacity-0 group-hover:opacity-40" />
          ))}
      </span>
    </th>
  )

  const allChecked = tools.length > 0 && tools.every((t) => selectedToolIds.has(t.id))

  return (
    <div className="overflow-x-auto">
      <table className="w-full min-w-[960px] border-collapse">
        <thead className="sticky top-0 z-10">
          <tr>
            <th className="w-10 px-3 py-3 bg-brand-50">
              <button
                onClick={() => tools.forEach((t) => onSelectRow(t.id, !allChecked))}
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
            <th className="px-3 py-3 text-left text-caption text-ink-500 bg-brand-50">Ответственный</th>
            {headerCell('qty', 'Кол-во')}
            <th className="w-12 px-3 py-3 bg-brand-50" />
          </tr>
        </thead>
        <tbody>
          {sorted.map((tool, i) => {
            const checked = selectedToolIds.has(tool.id)
            return (
              <motion.tr
                key={tool.id}
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ duration: 0.2, delay: Math.min(i, 12) * 0.02 }}
                onClick={() => navigate(`/tool/${tool.numericId}`)}
                className={cn(
                  'h-14 border-b border-brand-100/70 cursor-pointer transition-colors duration-120 hover:bg-brand-50',
                  checked && 'bg-brand-50/70'
                )}
              >
                <td className="px-3" onClick={(e) => e.stopPropagation()}>
                  <button
                    onClick={() => onSelectRow(tool.id, !checked)}
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
                  <img
                    src={tool.photo}
                    alt=""
                    className="w-10 h-10 rounded-lg object-cover border border-brand-100/60"
                    loading="lazy"
                  />
                </td>
                <td className="px-3 font-mono-num text-ink-500 whitespace-nowrap">
                  <Highlight text={tool.vn} query={query} />
                </td>
                <td className="px-3 text-sm font-semibold text-ink-900 max-w-[260px]">
                  <span className="line-clamp-1">
                    <Highlight text={tool.name} query={query} />
                  </span>
                </td>
                <td className="px-3 text-[13px] text-ink-500 whitespace-nowrap">
                  {tool.categoryName}
                </td>
                <td className="px-3">
                  <span
                    className="inline-flex items-center rounded-full px-2.5 py-0.5 text-caption"
                    style={{ background: tool.statusBg, color: tool.statusColor }}
                  >
                    {tool.statusName}
                  </span>
                </td>
                <td className="px-3 text-[13px] text-ink-500 whitespace-nowrap">
                  {tool.siteName ?? '—'}
                </td>
                <td className="px-3 text-[13px] text-ink-500 whitespace-nowrap">
                  {tool.warehouseName ?? '—'}
                </td>
                <td className="px-3">
                  {tool.assigneeName ? (
                    <span className="inline-flex items-center gap-2">
                      {tool.assigneeAvatar && (
                        <img
                          src={tool.assigneeAvatar}
                          alt=""
                          className="w-6 h-6 rounded-full object-cover border border-brand-100"
                        />
                      )}
                      <span className="text-[13px] font-semibold text-ink-900 whitespace-nowrap">{tool.assigneeName}</span>
                    </span>
                  ) : (
                    <span className="text-[13px] text-ink-300">На складе</span>
                  )}
                </td>
                <td className="px-3 font-mono-num text-ink-900">
                  {tool.isMaterial && typeof tool.quantity === 'number' ? `${tool.quantity} ${tool.unit}` : ''}
                </td>
                <td className="px-3 relative" onClick={(e) => e.stopPropagation()}>
                  <button
                    onClick={() => setMenuFor(menuFor === tool.id ? null : tool.id)}
                    className="w-8 h-8 flex items-center justify-center rounded-lg text-ink-300 hover:bg-brand-50 hover:text-ink-900 transition-colors"
                  >
                    <MoreVertical size={16} />
                  </button>
                  {menuFor === tool.id && (
                    <>
                      <div className="fixed inset-0 z-40" onClick={() => setMenuFor(null)} />
                      <div className="absolute right-3 top-full mt-1 w-44 rounded-xl border border-brand-100 bg-surface p-1 shadow-hover z-50">
                        {[
                          { label: 'Открыть', action: () => navigate(`/tool/${tool.numericId}`) },
                          { label: 'Передать', action: () => navigate('/transfers') },
                          { label: 'Печать QR', action: () => setMenuFor(null) },
                          { label: 'Удалить', action: () => setMenuFor(null), danger: true },
                        ].map((item) => (
                          <button
                            key={item.label}
                            onClick={() => {
                              item.action()
                              setMenuFor(null)
                            }}
                            className={cn(
                              'w-full text-left rounded-lg px-3 py-2 text-sm font-semibold transition-colors',
                              item.danger ? 'text-danger hover:bg-danger-bg' : 'text-ink-900 hover:bg-brand-50'
                            )}
                          >
                            {item.label}
                          </button>
                        ))}
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
