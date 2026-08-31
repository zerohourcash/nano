import { useEffect, useMemo, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import {
  Users,
  TrendingUpDown,
  Boxes,
  FileOutput,
  FileText,
  Download,
  Mail,
  Printer,
  Search,
  Check,
  Loader2,
  FileSpreadsheet,
  Eye,
} from 'lucide-react'
import { endOfWeek, format, startOfDay, startOfMonth, startOfWeek, subDays } from 'date-fns'
import { ru } from 'date-fns/locale'
import { cn } from '@/lib/utils'
import { buildXlsx } from '@/lib/xlsx'
import { trpc } from '@/providers/trpc'
import { useStore } from '@/lib/store'
import type { inferRouterOutputs } from '@trpc/server'
import type { AppRouter } from '../../api/router'

// ─── Типы tRPC (вывод из роутера) ────────────────────────────────────────────

type RouterOutputs = inferRouterOutputs<AppRouter>
type ApiUserGroup = RouterOutputs['reports']['byUsers'][number]
type ApiTx = RouterOutputs['reports']['quantityTransactions'][number]
type ApiItem = RouterOutputs['reports']['allItems'][number]
type ApiUser = RouterOutputs['admin']['users']['list'][number]

// ─── View-модели ────────────────────────────────────────────────────────────

type ReportType = 'users' | 'quantity' | 'all'
type Grouping = 'day' | 'week' | 'month'

interface VItem {
  id: string
  vn: string
  title: string
  category: string
  statusName: string
  statusSlug: string | null
  cost: number | null
  serial: string | null
  hasQr: boolean
  place: string
  responsibleName: string
  quantitative: boolean
  quantity: number | null
  unit: string | null
}

interface VUserGroup {
  key: string
  userId: number | null
  name: string
  position: string | null
  avatar: string | null
  items: VItem[]
}

interface VTx {
  id: string
  date: Date
  kind: 'write_off' | 'replenish'
  itemTitle: string
  vn: string
  actor: string
  qty: number | null
  comment: string | null
}

interface VEmployee {
  key: string
  name: string
  position: string | null
  avatar: string | null
}

interface HistoryEntryUI {
  id: string
  type: ReportType
  name: string
  date: Date
  author: string
  formatLabel: 'CSV' | 'XLSX' | 'PDF'
}

// ─── Адаптеры tRPC → view-модели ────────────────────────────────────────────

function adaptApiItem(it: ApiItem): VItem {
  return {
    id: String(it.id),
    vn: it.internalId,
    title: it.title,
    category: it.category?.name ?? 'Без категории',
    statusName: it.status?.name ?? '—',
    statusSlug: it.status?.slug ?? null,
    cost: it.cost ?? null,
    serial: it.serialNumber ?? null,
    hasQr: Boolean(it.qrCode),
    place: it.buildingSite?.name ?? it.storage?.name ?? '—',
    responsibleName: it.responsible?.fullName ?? '—',
    quantitative: it.quantitative,
    quantity: it.quantity ?? null,
    unit: it.unit ?? null,
  }
}

function adaptUserGroups(rows: ApiUserGroup[]): VUserGroup[] {
  return rows.map((g) => ({
    key: g.userId === null ? 'none' : String(g.userId),
    userId: g.userId,
    name: g.user?.fullName ?? 'Без ответственного',
    position: g.user?.position ?? null,
    avatar: g.user?.avatarUrl ?? null,
    items: g.items.map((it) => ({
      id: String(it.id),
      vn: it.internalId,
      title: it.title,
      category: it.category?.name ?? 'Без категории',
      statusName: it.status?.name ?? '—',
      statusSlug: it.status?.slug ?? null,
      cost: it.cost ?? null,
      serial: null,
      hasQr: false,
      place: '—',
      responsibleName: g.user?.fullName ?? '—',
      quantitative: it.quantitative,
      quantity: it.quantity ?? null,
      unit: it.unit ?? null,
    })),
  }))
}

function adaptTx(rows: ApiTx[]): VTx[] {
  return rows.map((h) => ({
    id: String(h.id),
    date: new Date(h.createdAt),
    kind: h.type === 'replenish' ? 'replenish' : 'write_off',
    itemTitle: h.item?.title ?? '—',
    vn: h.item?.internalId ?? '—',
    actor: h.actor?.fullName ?? '—',
    qty: h.quantityDelta === null || h.quantityDelta === undefined ? null : Math.abs(h.quantityDelta),
    comment: h.comment ?? null,
  }))
}

function adaptEmployees(rows: ApiUser[]): VEmployee[] {
  return rows.map((u) => ({
    key: String(u.id),
    name: u.fullName,
    position: u.position ?? null,
    avatar: u.avatarUrl ?? null,
  }))
}

// ─── Утилиты ─────────────────────────────────────────────────────────────────

const fmtMoney = (v: number) =>
  `${new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 0 }).format(v)} ₽`

const fmtNum = (v: number) => new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 2 }).format(v)

function groupTxs(txs: VTx[], g: Grouping) {
  const map = new Map<string, { sort: number; label: string; inQty: number; outQty: number; ops: number }>()
  for (const t of txs) {
    const d = t.date
    let key: string
    let label: string
    let sort: number
    if (g === 'day') {
      const s = startOfDay(d)
      key = format(s, 'yyyy-MM-dd')
      label = format(s, 'dd.MM.yyyy')
      sort = s.getTime()
    } else if (g === 'week') {
      const ws = startOfWeek(d, { weekStartsOn: 1 })
      const we = endOfWeek(d, { weekStartsOn: 1 })
      key = format(ws, 'yyyy-MM-dd')
      label = `${format(ws, 'dd.MM')} – ${format(we, 'dd.MM.yyyy')}`
      sort = ws.getTime()
    } else {
      const ms = startOfMonth(d)
      key = format(ms, 'yyyy-MM')
      label = format(ms, 'LLLL yyyy', { locale: ru })
      sort = ms.getTime()
    }
    const row = map.get(key) ?? { sort, label, inQty: 0, outQty: 0, ops: 0 }
    if (t.kind === 'replenish') row.inQty += t.qty ?? 0
    else row.outQty += t.qty ?? 0
    row.ops += 1
    map.set(key, row)
  }
  return [...map.values()].sort((a, b) => b.sort - a.sort)
}

function csvEscape(v: string | number): string {
  let s = String(v)
  // Excel исполняет ячейку, начинающуюся с =, +, - или @ — экранируем.
  if (/^[=+\-@\t\r]/.test(s)) s = `'${s}`
  return `"${s.replace(/"/g, '""')}"`
}

function htmlEscape(v: string): string {
  return v.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

function downloadFile(filename: string, content: string, mime: string) {
  const blob = new Blob([content], { type: `${mime};charset=utf-8` })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = filename
  document.body.appendChild(a)
  a.click()
  a.remove()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}

// ─── Мелкие UI-атомы ─────────────────────────────────────────────────────────

/** Кастомный чекбокс 20px (design.md §6) */
function CheckBox({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean
  onChange: (v: boolean) => void
  label?: React.ReactNode
  disabled?: boolean
}) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        'flex items-center gap-2.5 text-left group disabled:opacity-50',
        label ? '' : 'shrink-0'
      )}
    >
      <span
        className={cn(
          'w-5 h-5 rounded-md border-[1.5px] flex items-center justify-center transition-colors duration-150 shrink-0',
          checked ? 'bg-brand-600 border-brand-600' : 'border-brand-100 bg-white group-hover:border-brand-600/50'
        )}
      >
        <motion.span
          initial={false}
          animate={{ scale: checked ? 1 : 0, opacity: checked ? 1 : 0 }}
          transition={{ duration: 0.15 }}
          className="flex"
        >
          <Check size={13} strokeWidth={3} className="text-white" />
        </motion.span>
      </span>
      {label}
    </button>
  )
}

function Avatar({ name, src, size = 32 }: { name: string; src: string | null; size?: number }) {
  const initials = name
    .split(' ')
    .map((w) => w[0])
    .slice(0, 2)
    .join('')
    .toUpperCase()
  if (src) {
    return (
      <img
        src={src}
        alt={name}
        style={{ width: size, height: size }}
        className="rounded-full object-cover border border-brand-100/60 shrink-0"
      />
    )
  }
  return (
    <span
      style={{ width: size, height: size, fontSize: size * 0.38 }}
      className="rounded-full bg-brand-50 text-brand-600 font-semibold flex items-center justify-center shrink-0"
    >
      {initials}
    </span>
  )
}

function Toast({ text }: { text: string | null }) {
  return (
    <AnimatePresence>
      {text && (
        <motion.div
          initial={{ opacity: 0, y: 24 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 12 }}
          transition={{ duration: 0.22, ease: 'easeOut' }}
          className="fixed bottom-20 lg:bottom-6 left-1/2 -translate-x-1/2 z-[70] flex items-center gap-2 rounded-full bg-ink-900 text-white text-sm font-semibold px-5 py-2.5 shadow-modal"
        >
          <Check size={16} className="text-teal" />
          {text}
        </motion.div>
      )}
    </AnimatePresence>
  )
}

// ─── Константы ───────────────────────────────────────────────────────────────

const REPORT_TYPES: { type: ReportType; icon: typeof Users; title: string; desc: string }[] = [
  {
    type: 'users',
    icon: Users,
    title: 'Ответственные',
    desc: 'Кто за что отвечает: имущество в разрезе сотрудников, сумма на каждом',
  },
  {
    type: 'quantity',
    icon: TrendingUpDown,
    title: 'Поступление / Списание',
    desc: 'Движение ТМЦ за период: что пришло и ушло, по категориям и суммам',
  },
  {
    type: 'all',
    icon: Boxes,
    title: 'Все имущество',
    desc: 'Полная опись: каждая единица с местом, ответственным и стоимостью',
  },
]

const REPORT_TITLES: Record<ReportType, string> = {
  users: 'Ответственные',
  quantity: 'Поступление/Списание',
  all: 'Все имущество',
}

const DEMO_HISTORY: HistoryEntryUI[] = [
  {
    id: 'rh-1',
    type: 'quantity',
    name: 'Поступление/Списание, февраль 2025',
    date: new Date('2025-03-03T11:20:00'),
    author: 'Алексей Кузнецов',
    formatLabel: 'XLSX',
  },
  {
    id: 'rh-2',
    type: 'all',
    name: 'Все имущество, февраль 2025',
    date: new Date('2025-02-28T17:48:00'),
    author: 'Ольга Демидова',
    formatLabel: 'PDF',
  },
  {
    id: 'rh-3',
    type: 'users',
    name: 'Ответственные, Q1 2025',
    date: new Date('2025-02-15T10:05:00'),
    author: 'Алексей Кузнецов',
    formatLabel: 'XLSX',
  },
  {
    id: 'rh-4',
    type: 'quantity',
    name: 'Поступление/Списание, январь 2025',
    date: new Date('2025-02-02T09:31:00'),
    author: 'Марина Орлова',
    formatLabel: 'PDF',
  },
]

// ─── Страница ────────────────────────────────────────────────────────────────

export default function Reports() {
  const { workspace, currentUser } = useStore()
  const [type, setType] = useState<ReportType>('users')

  // Конфигуратор «Ответственные»
  const [empSearch, setEmpSearch] = useState('')
  const [selectedEmps, setSelectedEmps] = useState<Set<string> | null>(null) // null = все
  const [includeWrittenOff, setIncludeWrittenOff] = useState(false)

  // Конфигуратор «Поступление/Списание»
  const [dateFrom, setDateFrom] = useState('')
  const [dateTo, setDateTo] = useState('')
  const [grouping, setGrouping] = useState<Grouping>('day')
  const [showDateErrors, setShowDateErrors] = useState(false)

  // Конфигуратор «Все имущество»
  const [cols, setCols] = useState({ cost: true, serial: true, qr: true })
  const [statusFilter, setStatusFilter] = useState<Set<string>>(new Set())

  // Предпросмотр / история / тосты
  const [previewOpen, setPreviewOpen] = useState(false)
  const [previewStamp, setPreviewStamp] = useState<Date | null>(null)
  const [forming, setForming] = useState(false)
  const [historyList, setHistoryList] = useState<HistoryEntryUI[]>(DEMO_HISTORY)
  const [toast, setToast] = useState<string | null>(null)
  const toastTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const previewRef = useRef<HTMLDivElement>(null)

  const showToast = (msg: string) => {
    setToast(msg)
    if (toastTimer.current) clearTimeout(toastTimer.current)
    toastTimer.current = setTimeout(() => setToast(null), 4000)
  }
  useEffect(() => () => {
    if (toastTimer.current) clearTimeout(toastTimer.current)
  }, [])

  // ─── Данные tRPC ───────────────────────────────────────────────────────────

  const usersQ = trpc.reports.byUsers.useQuery(undefined, { enabled: type === 'users', retry: 1 })
  const empsQ = trpc.admin.users.list.useQuery(undefined, { enabled: type === 'users', retry: 1 })
  const allQ = trpc.reports.allItems.useQuery(undefined, { enabled: type === 'all', retry: 1 })

  const parsedFrom = dateFrom ? new Date(`${dateFrom}T00:00:00`) : null
  const parsedTo = dateTo ? new Date(`${dateTo}T23:59:59.999`) : null
  const datesValid = Boolean(parsedFrom && parsedTo && parsedFrom.getTime() <= parsedTo.getTime())
  const dateError = !dateFrom || !dateTo
    ? 'Укажите обе даты диапазона'
    : !datesValid
      ? 'Дата «С» не может быть позже даты «По»'
      : null

  const qtyQ = trpc.reports.quantityTransactions.useQuery(
    {
      dateFrom: parsedFrom ?? undefined,
      dateTo: parsedTo ?? undefined,
    },
    { enabled: type === 'quantity' && datesValid, retry: 1 }
  )

  // ─── Нормализация данных ──────────────────────────────────────────────────
  // Отчёт по оборудованию нельзя подменять выдуманными строками: при ошибке
  // запроса показываем пустой отчёт и явное предупреждение.

  const userGroups: VUserGroup[] = useMemo(
    () => (usersQ.data ? adaptUserGroups(usersQ.data) : []),
    [usersQ.data],
  )

  const employees: VEmployee[] = useMemo(
    () => (empsQ.data ? adaptEmployees(empsQ.data) : []),
    [empsQ.data],
  )

  const allItems: VItem[] = useMemo(
    () => (allQ.data ? allQ.data.map(adaptApiItem) : []),
    [allQ.data],
  )

  const txs: VTx[] = useMemo(() => (qtyQ.data ? adaptTx(qtyQ.data) : []), [qtyQ.data])

  const isLoading =
    (type === 'users' && usersQ.isLoading) ||
    (type === 'all' && allQ.isLoading) ||
    (type === 'quantity' && qtyQ.isLoading && datesValid)

  const loadError =
    (type === 'users' && (usersQ.error ?? empsQ.error)) ||
    (type === 'all' && allQ.error) ||
    (type === 'quantity' && qtyQ.error) ||
    null
  const retryLoad = () => {
    if (type === 'users') {
      void usersQ.refetch()
      void empsQ.refetch()
    } else if (type === 'all') void allQ.refetch()
    else void qtyQ.refetch()
  }

  // ─── Применение конфигуратора ─────────────────────────────────────────────

  const toggleEmp = (key: string) => {
    const allKeys = [...employees.map((e) => e.key), 'none']
    const cur = selectedEmps === null ? new Set(allKeys) : new Set(selectedEmps)
    if (cur.has(key)) cur.delete(key)
    else cur.add(key)
    setSelectedEmps(cur.size === allKeys.length ? null : cur)
  }

  const filteredGroups = useMemo(
    () =>
      userGroups
        .filter((g) => selectedEmps === null || selectedEmps.has(g.key))
        .map((g) => ({
          ...g,
          items: includeWrittenOff ? g.items : g.items.filter((i) => i.statusSlug !== 'written-off'),
        })),
    [userGroups, selectedEmps, includeWrittenOff]
  )

  const statusOptions = useMemo(() => {
    const m = new Map<string, string>()
    for (const it of allItems) m.set(it.statusSlug ?? it.statusName, it.statusName)
    return [...m.entries()].map(([slug, name]) => ({ slug, name }))
  }, [allItems])

  const filteredItems = useMemo(
    () => allItems.filter((i) => statusFilter.size === 0 || statusFilter.has(i.statusSlug ?? i.statusName)),
    [allItems, statusFilter]
  )

  const groupedTxs = useMemo(() => groupTxs(txs, grouping), [txs, grouping])

  // ─── Сводка и гистограмма ─────────────────────────────────────────────────

  const previewItems: VItem[] = useMemo(() => {
    if (type === 'users') return filteredGroups.flatMap((g) => g.items)
    if (type === 'all') return filteredItems
    return []
  }, [type, filteredGroups, filteredItems])

  const totalCost = previewItems.reduce((s, i) => s + (i.cost ?? 0), 0)

  const catDist = useMemo(() => {
    const m = new Map<string, number>()
    for (const it of previewItems) m.set(it.category, (m.get(it.category) ?? 0) + 1)
    const rows = [...m.entries()].sort((a, b) => b[1] - a[1])
    const top = rows.slice(0, 5)
    const rest = rows.slice(5).reduce((s, r) => s + r[1], 0)
    if (rest > 0) top.push(['Прочее', rest])
    const total = previewItems.length || 1
    return top.map(([name, count]) => ({ name, count, pct: Math.round((count / total) * 100) }))
  }, [previewItems])

  const stats: { value: string; label: string }[] = useMemo(() => {
    if (type === 'users') {
      return [
        { value: String(filteredGroups.reduce((s, g) => s + g.items.length, 0)), label: 'единиц' },
        { value: fmtMoney(totalCost), label: 'суммарно' },
        { value: String(filteredGroups.length), label: 'ответственных' },
      ]
    }
    if (type === 'quantity') {
      const inOps = txs.filter((t) => t.kind === 'replenish')
      const outOps = txs.filter((t) => t.kind === 'write_off')
      return [
        { value: fmtNum(inOps.reduce((s, t) => s + (t.qty ?? 0), 0)), label: 'поступило (шт/ед)' },
        { value: fmtNum(outOps.reduce((s, t) => s + (t.qty ?? 0), 0)), label: 'списано (шт/ед)' },
        { value: String(txs.length), label: 'операций' },
      ]
    }
    return [
      { value: String(filteredItems.length), label: 'единиц' },
      { value: fmtMoney(totalCost), label: 'суммарно' },
      { value: String(new Set(filteredItems.map((i) => i.category)).size), label: 'категорий' },
    ]
  }, [type, filteredGroups, filteredItems, txs, totalCost])

  // ─── Таблица результата (предпросмотр + выгрузка) ─────────────────────────

  const tableData = useMemo((): { headers: string[]; rows: (string | number)[][] } => {
    if (type === 'users') {
      return {
        headers: ['Ответственный', 'Должность', 'Единиц', 'Сумма'],
        rows: filteredGroups.map((g) => [
          g.name,
          g.position ?? '—',
          g.items.length,
          g.items.reduce((s, i) => s + (i.cost ?? 0), 0),
        ]),
      }
    }
    if (type === 'quantity') {
      return {
        headers: ['Период', 'Поступление', 'Списание', 'Операций'],
        rows: groupedTxs.map((r) => [r.label, fmtNum(r.inQty), fmtNum(r.outQty), r.ops]),
      }
    }
    const headers = ['Вн. номер', 'Название', 'Категория', 'Статус', 'Место', 'Ответственный']
    if (cols.cost) headers.push('Стоимость')
    if (cols.serial) headers.push('Серийный номер')
    if (cols.qr) headers.push('QR-метка')
    return {
      headers,
      rows: filteredItems.map((i) => {
        const row: (string | number)[] = [i.vn, i.title, i.category, i.statusName, i.place, i.responsibleName]
        if (cols.cost) row.push(i.cost ?? '—')
        if (cols.serial) row.push(i.serial ?? '—')
        if (cols.qr) row.push(i.hasQr ? 'есть' : '—')
        return row
      }),
    }
  }, [type, filteredGroups, groupedTxs, filteredItems, cols])

  // ─── Действия ──────────────────────────────────────────────────────────────

  const openPreview = (stamp?: Date) => {
    setPreviewOpen(true)
    setPreviewStamp(stamp ?? new Date())
    setTimeout(() => previewRef.current?.scrollIntoView({ behavior: 'smooth', block: 'start' }), 80)
  }

  const handleForm = () => {
    if (type === 'quantity' && !datesValid) {
      setShowDateErrors(true)
      return
    }
    setForming(true)
    setTimeout(() => {
      setForming(false)
      openPreview()
    }, 600)
  }

  const fileBase = () => `${REPORT_TITLES[type]}_${format(new Date(), 'yyyy-MM-dd_HH-mm')}`

  const pushHistory = (formatLabel: HistoryEntryUI['formatLabel']) => {
    setHistoryList((prev) => [
      {
        id: `rh-${Date.now()}`,
        type,
        name: `${REPORT_TITLES[type]}, ${format(new Date(), 'LLLL yyyy', { locale: ru })}`,
        date: new Date(),
        author: currentUser?.fullName ?? '—',
        formatLabel,
      },
      ...prev,
    ])
  }

  const handleCsv = () => {
    const { headers, rows } = tableData
    const csv = [headers, ...rows].map((r) => r.map(csvEscape).join(';')).join('\r\n')
    downloadFile(`${fileBase()}.csv`, '\uFEFF' + csv, 'text/csv')
    pushHistory('CSV')
    showToast('Отчёт сохранён')
  }

  const handleXls = () => {
    const { headers, rows } = tableData
    // Настоящий xlsx, а не HTML-таблица с чужим расширением: числа остаются
    // числами, и файл открывается не только в Excel.
    const blob = buildXlsx([headers, ...rows], REPORT_TITLES[type])
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = `${fileBase()}.xlsx`
    document.body.appendChild(a)
    a.click()
    a.remove()
    setTimeout(() => URL.revokeObjectURL(url), 1000)
    pushHistory('XLSX')
    showToast('Отчёт сохранён')
  }

  const handlePdf = () => {
    const { headers, rows } = tableData
    const w = window.open('', '_blank', 'width=900,height=700')
    if (!w) return
    const cells = rows
      .map(
        (r) =>
          `<tr>${r
            .map(
              (c) =>
                `<td style="border:1px solid #C9C9F0;padding:6px 8px;font-size:12px">${htmlEscape(String(c))}</td>`,
            )
            .join('')}</tr>`,
      )
      .join('')
    w.document.write(
      `<html><head><title>${REPORT_TITLES[type]}</title><meta charset="utf-8"></head><body style="font-family:Segoe UI,sans-serif;padding:24px">` +
        `<h1 style="font-size:20px">${htmlEscape(REPORT_TITLES[type])}</h1>` +
        `<p>${htmlEscape(workspace?.name ?? '')} · ${format(new Date(), 'dd.MM.yyyy HH:mm')}</p>` +
        `<table style="border-collapse:collapse;width:100%"><thead><tr>${headers
          .map((h) => `<th style="border:1px solid #C9C9F0;padding:6px 8px;background:#EDEDF7;text-align:left">${htmlEscape(h)}</th>`)
          .join('')}</tr></thead><tbody>${cells}</tbody></table></body></html>`,
    )
    w.document.close()
    w.focus()
    w.print()
    pushHistory('PDF')
    showToast('Отчёт отправлен на печать / PDF')
  }

  const handleEmail = () => {
    const subject = encodeURIComponent(`Отчёт Everyday: ${REPORT_TITLES[type]} · ${workspace?.name ?? ''}`)
    const body = encodeURIComponent(
      `Отчёт «${REPORT_TITLES[type]}»\nРабочее пространство: ${workspace?.name ?? ''}\nСформирован: ${format(new Date(), 'dd.MM.yyyy HH:mm')}\n\n${stats.map((s) => `${s.label}: ${s.value}`).join('\n')}`
    )
    window.location.href = `mailto:?subject=${subject}&body=${body}`
  }

  const applyChip = (days: number) => {
    const to = new Date()
    setDateFrom(format(subDays(to, days), 'yyyy-MM-dd'))
    setDateTo(format(to, 'yyyy-MM-dd'))
    setShowDateErrors(false)
  }

  // ─── Render ────────────────────────────────────────────────────────────────

  const visibleEmployees = employees.filter((e) =>
    e.name.toLowerCase().includes(empSearch.trim().toLowerCase())
  )
  const hasUnassigned = userGroups.some((g) => g.key === 'none')

  return (
    <div className="space-y-5">
      {/* Секция 1. Заголовок */}
      <motion.div initial={{ opacity: 0, y: 16 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.28 }}>
        <h1 className="text-2xl lg:text-[28px] leading-9 font-bold tracking-[-0.01em] text-ink-900">Отчёты</h1>
        <p className="text-[13px] leading-[18px] text-ink-500 mt-1">
          Аналитика имущества рабочего пространства
        </p>
      </motion.div>

      {loadError && (
        <div className="rounded-card border border-danger/30 bg-danger/5 p-4 text-sm text-danger">
          <span className="font-semibold">Отчёт не загружен.</span> {loadError.message}
          <button type="button" onClick={retryLoad} className="ml-2 font-semibold underline">
            Повторить
          </button>
        </div>
      )}

      {/* Секция 2. Карточки типов отчётов */}
      <div className="flex gap-3 overflow-x-auto pb-1 snap-x lg:grid lg:grid-cols-3 lg:overflow-visible">
        {REPORT_TYPES.map((rt, idx) => {
          const active = type === rt.type
          const Icon = rt.icon
          return (
            <motion.button
              key={rt.type}
              initial={{ opacity: 0, y: 20 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.28, delay: idx * 0.07 }}
              whileHover={active ? undefined : { y: -2 }}
              whileTap={{ scale: 0.98 }}
              onClick={() => setType(rt.type)}
              className={cn(
                'min-w-[260px] snap-start lg:min-w-0 text-left bg-surface rounded-card border-2 shadow-card p-5 lg:p-6 transition-[border-color,box-shadow] duration-200',
                active ? 'border-accent bg-[#E0235B08] scale-[1.01]' : 'border-transparent hover:shadow-hover'
              )}
            >
              <span
                className={cn(
                  'inline-flex w-11 h-11 rounded-xl items-center justify-center transition-colors duration-200',
                  active ? 'bg-accent text-white' : 'bg-brand-50 text-brand-600'
                )}
              >
                <Icon size={22} strokeWidth={1.75} />
              </span>
              <span className="block mt-3 text-[17px] leading-6 font-semibold text-ink-900">{rt.title}</span>
              <span className="block mt-1 text-[13px] leading-[18px] text-ink-500">{rt.desc}</span>
            </motion.button>
          )
        })}
      </div>

      {/* Секция 3. Конфигуратор */}
      <motion.div layout className="bg-surface rounded-card border border-brand-100/60 shadow-card p-5 lg:p-6">
        <AnimatePresence mode="wait" initial={false}>
          <motion.div
            key={type}
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
          >
            {type === 'users' && (
              <div className="space-y-4">
                <h3 className="text-[17px] leading-6 font-semibold text-ink-900">Сотрудники в отчёте</h3>
                <div className="relative max-w-sm">
                  <Search size={16} className="absolute left-3.5 top-1/2 -translate-y-1/2 text-ink-300" />
                  <input
                    value={empSearch}
                    onChange={(e) => setEmpSearch(e.target.value)}
                    placeholder="Поиск по фамилии"
                    className="h-11 w-full rounded-xl border border-brand-100 bg-white pl-10 pr-3 text-sm text-ink-900 placeholder:text-ink-300 focus:border-brand-600 focus:shadow-[0_0_0_3px_#5E629B22] transition-shadow"
                  />
                </div>
                <div className="grid sm:grid-cols-2 lg:grid-cols-3 gap-2">
                  {visibleEmployees.map((e) => (
                    <div
                      key={e.key}
                      className="flex items-center gap-2.5 rounded-xl border border-brand-100/60 px-3 py-2.5 hover:bg-brand-50/50 transition-colors"
                    >
                      <CheckBox
                        checked={selectedEmps === null || selectedEmps.has(e.key)}
                        onChange={() => toggleEmp(e.key)}
                      />
                      <Avatar name={e.name} src={e.avatar} size={32} />
                      <span className="min-w-0">
                        <span className="block text-sm font-semibold text-ink-900 truncate">{e.name}</span>
                        <span className="block text-xs text-ink-500 truncate">{e.position ?? '—'}</span>
                      </span>
                    </div>
                  ))}
                  {hasUnassigned && !empSearch.trim() && (
                    <div className="flex items-center gap-2.5 rounded-xl border border-dashed border-brand-100 px-3 py-2.5 hover:bg-brand-50/50 transition-colors">
                      <CheckBox
                        checked={selectedEmps === null || selectedEmps.has('none')}
                        onChange={() => toggleEmp('none')}
                      />
                      <Avatar name="?" src={null} size={32} />
                      <span className="min-w-0">
                        <span className="block text-sm font-semibold text-ink-500 truncate">Без ответственного</span>
                        <span className="block text-xs text-ink-300 truncate">не назначены</span>
                      </span>
                    </div>
                  )}
                </div>
                {visibleEmployees.length === 0 && (
                  <p className="text-[13px] text-ink-500">Никого не найдено по запросу «{empSearch}»</p>
                )}
                <div className="flex flex-wrap items-center gap-x-6 gap-y-2 pt-1">
                  <CheckBox
                    checked={includeWrittenOff}
                    onChange={setIncludeWrittenOff}
                    label={<span className="text-sm text-ink-900">Включать списанное имущество</span>}
                  />
                  <button
                    onClick={() => setSelectedEmps(null)}
                    className="text-sm font-semibold text-brand-600 hover:underline"
                  >
                    Выбрать всех
                  </button>
                </div>
              </div>
            )}

            {type === 'quantity' && (
              <div className="space-y-4">
                <h3 className="text-[17px] leading-6 font-semibold text-ink-900">
                  Диапазон дат <span className="text-accent">*</span>
                </h3>
                <div className="flex flex-wrap items-end gap-3">
                  <label className="block">
                    <span className="block text-[13px] font-semibold text-ink-500 mb-1.5">С</span>
                    <input
                      type="date"
                      value={dateFrom}
                      onChange={(e) => {
                        setDateFrom(e.target.value)
                        setShowDateErrors(false)
                      }}
                      className={cn(
                        'h-11 rounded-xl border bg-white px-3.5 text-sm text-ink-900 font-mono focus:border-brand-600 focus:shadow-[0_0_0_3px_#5E629B22] transition-shadow',
                        showDateErrors && dateError ? 'border-danger' : 'border-brand-100'
                      )}
                    />
                  </label>
                  <label className="block">
                    <span className="block text-[13px] font-semibold text-ink-500 mb-1.5">По</span>
                    <input
                      type="date"
                      value={dateTo}
                      onChange={(e) => {
                        setDateTo(e.target.value)
                        setShowDateErrors(false)
                      }}
                      className={cn(
                        'h-11 rounded-xl border bg-white px-3.5 text-sm text-ink-900 font-mono focus:border-brand-600 focus:shadow-[0_0_0_3px_#5E629B22] transition-shadow',
                        showDateErrors && dateError ? 'border-danger' : 'border-brand-100'
                      )}
                    />
                  </label>
                  <div className="flex gap-2 pb-0.5">
                    {(
                      [
                        ['Месяц', 30],
                        ['Квартал', 90],
                        ['Год', 365],
                      ] as [string, number][]
                    ).map(([label, days]) => (
                      <button
                        key={label}
                        onClick={() => applyChip(days)}
                        className="h-8 px-3.5 rounded-full border border-brand-100 bg-white text-[13px] font-semibold text-ink-900 hover:bg-brand-50 transition"
                      >
                        {label}
                      </button>
                    ))}
                  </div>
                </div>
                {showDateErrors && dateError && <p className="text-xs font-semibold text-danger">{dateError}</p>}
                <div>
                  <span className="block text-[13px] font-semibold text-ink-500 mb-1.5">Группировка</span>
                  <div className="inline-flex rounded-xl border border-brand-100 bg-white p-1">
                    {(
                      [
                        ['day', 'По дням'],
                        ['week', 'По неделям'],
                        ['month', 'По месяцам'],
                      ] as [Grouping, string][]
                    ).map(([g, label]) => (
                      <button
                        key={g}
                        onClick={() => setGrouping(g)}
                        className={cn(
                          'h-8 px-3.5 rounded-lg text-[13px] font-semibold transition-colors',
                          grouping === g ? 'bg-brand-600 text-white' : 'text-ink-500 hover:text-ink-900'
                        )}
                      >
                        {label}
                      </button>
                    ))}
                  </div>
                </div>
              </div>
            )}

            {type === 'all' && (
              <div className="space-y-4">
                <h3 className="text-[17px] leading-6 font-semibold text-ink-900">Фильтры и колонки</h3>
                <div>
                  <span className="block text-[13px] font-semibold text-ink-500 mb-1.5">Статусы</span>
                  <div className="flex flex-wrap gap-2">
                    {statusOptions.map((s) => {
                      const active = statusFilter.has(s.slug)
                      return (
                        <button
                          key={s.slug}
                          onClick={() =>
                            setStatusFilter((prev) => {
                              const next = new Set(prev)
                              if (next.has(s.slug)) next.delete(s.slug)
                              else next.add(s.slug)
                              return next
                            })
                          }
                          className={cn(
                            'h-8 px-3.5 rounded-full border text-[13px] font-semibold transition',
                            active
                              ? 'border-brand-600 bg-brand-600 text-white'
                              : 'border-brand-100 bg-white text-ink-900 hover:bg-brand-50'
                          )}
                        >
                          {s.name}
                        </button>
                      )
                    })}
                    {statusFilter.size > 0 && (
                      <button
                        onClick={() => setStatusFilter(new Set())}
                        className="h-8 px-3 text-[13px] font-semibold text-brand-600 hover:underline"
                      >
                        Сбросить
                      </button>
                    )}
                  </div>
                </div>
                <div>
                  <span className="block text-[13px] font-semibold text-ink-500 mb-1.5">Колонки отчёта</span>
                  <div className="flex flex-wrap gap-x-6 gap-y-2">
                    <CheckBox
                      checked={cols.cost}
                      onChange={(v) => setCols((c) => ({ ...c, cost: v }))}
                      label={<span className="text-sm text-ink-900">Стоимость</span>}
                    />
                    <CheckBox
                      checked={cols.serial}
                      onChange={(v) => setCols((c) => ({ ...c, serial: v }))}
                      label={<span className="text-sm text-ink-900">Серийные номера</span>}
                    />
                    <CheckBox
                      checked={cols.qr}
                      onChange={(v) => setCols((c) => ({ ...c, qr: v }))}
                      label={<span className="text-sm text-ink-900">QR-метки</span>}
                    />
                  </div>
                </div>
              </div>
            )}
          </motion.div>
        </AnimatePresence>

        {/* Футер конфигуратора */}
        <div className="flex flex-wrap gap-3 mt-6 pt-5 border-t border-brand-100/60">
          <button
            onClick={handleForm}
            disabled={forming}
            className="inline-flex items-center gap-2 h-10 px-5 rounded-xl bg-accent text-white text-sm font-semibold hover:bg-accent-hover hover:-translate-y-px active:scale-[0.97] transition disabled:opacity-70"
          >
            {forming ? <Loader2 size={16} className="animate-spin" /> : <FileOutput size={16} />}
            {forming ? 'Формируем…' : 'Сформировать отчёт'}
          </button>
          <button
            onClick={() => {
              if (type === 'quantity' && !datesValid) {
                setShowDateErrors(true)
                return
              }
              openPreview()
            }}
            className="inline-flex items-center gap-2 h-10 px-4 rounded-xl text-sm font-semibold text-brand-600 hover:bg-brand-50 transition-colors"
          >
            <Eye size={16} />
            Предпросмотр
          </button>
        </div>
      </motion.div>

      {/* Секция 4. Предпросмотр */}
      <AnimatePresence>
        {previewOpen && (
          <motion.div
            ref={previewRef}
            initial={{ opacity: 0, y: 24 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: 12 }}
            transition={{ duration: 0.3, ease: 'easeOut' }}
            className="bg-surface rounded-card border border-brand-100/60 shadow-card p-5 lg:p-6 space-y-5 scroll-mt-20"
          >
            <div className="flex flex-wrap items-baseline justify-between gap-2">
              <h3 className="text-[17px] leading-6 font-semibold text-ink-900">
                {REPORT_TITLES[type]} · {workspace?.name}
              </h3>
              {previewStamp && (
                <span className="font-mono-num text-ink-500">
                  сформирован {format(previewStamp, 'dd.MM.yyyy HH:mm')}
                </span>
              )}
            </div>

            {/* Мини-статы */}
            <div className="grid grid-cols-3 gap-3">
              {stats.map((s, i) => (
                <motion.div
                  key={s.label}
                  initial={{ opacity: 0, y: 12 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ duration: 0.25, delay: i * 0.05 }}
                  className="rounded-xl bg-brand-50/70 border border-brand-100/50 px-4 py-3"
                >
                  <div className="text-lg lg:text-xl font-bold text-ink-900 truncate">{s.value}</div>
                  <div className="text-[11px] font-semibold text-ink-500 uppercase tracking-wide">{s.label}</div>
                </motion.div>
              ))}
            </div>

            {/* Гистограмма по категориям (не для «Поступление/Списание») */}
            {type !== 'quantity' && catDist.length > 0 && (
              <div className="space-y-2">
                <span className="text-[13px] font-semibold text-ink-500">Распределение по категориям</span>
                {catDist.map((c, i) => (
                  <div key={c.name} className="flex items-center gap-3">
                    <span className="w-36 sm:w-44 shrink-0 text-xs text-ink-500 truncate">{c.name}</span>
                    <div className="flex-1 h-2.5 rounded-full bg-brand-50 overflow-hidden">
                      <motion.div
                        initial={{ width: 0 }}
                        whileInView={{ width: `${c.pct}%` }}
                        viewport={{ once: true, amount: 0.6 }}
                        transition={{ duration: 0.7, delay: i * 0.08, ease: 'easeOut' }}
                        className="h-full rounded-full bg-brand-600"
                      />
                    </div>
                    <span className="w-10 shrink-0 text-right text-xs font-semibold text-ink-900">{c.pct}%</span>
                  </div>
                ))}
              </div>
            )}

            {/* Таблица результата */}
            <div className="rounded-xl border border-brand-100/60 overflow-hidden">
              <div className="max-h-[380px] overflow-auto">
                <table className="w-full text-sm">
                  <thead className="sticky top-0 z-10">
                    <tr className="bg-brand-50">
                      {tableData.headers.map((h) => (
                        <th
                          key={h}
                          className="px-4 py-2.5 text-left text-caption text-ink-500 whitespace-nowrap"
                        >
                          {h}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {isLoading ? (
                      <tr>
                        <td colSpan={tableData.headers.length} className="px-4 py-10 text-center text-ink-500">
                          <Loader2 size={18} className="inline animate-spin mr-2" />
                          Загрузка данных…
                        </td>
                      </tr>
                    ) : tableData.rows.length === 0 ? (
                      <tr>
                        <td colSpan={tableData.headers.length} className="px-4 py-10 text-center text-ink-500">
                          Нет данных для выбранной конфигурации
                        </td>
                      </tr>
                    ) : (
                      tableData.rows.map((row, ri) => (
                        <tr key={ri} className="border-t border-brand-100/60 hover:bg-brand-50 transition-colors">
                          {row.map((cell, ci) => (
                            <td
                              key={ci}
                              className={cn(
                                'px-4 py-3 text-ink-900 whitespace-nowrap',
                                (ci === 0 && type === 'all') || typeof cell === 'number' ? 'font-mono-num' : ''
                              )}
                            >
                              {typeof cell === 'number' &&
                              (tableData.headers[ci] === 'Сумма' || tableData.headers[ci] === 'Стоимость')
                                ? fmtMoney(cell)
                                : cell}
                            </td>
                          ))}
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
              </div>
            </div>

            {/* Действия */}
            <div className="flex flex-wrap gap-3">
              <button
                onClick={handleCsv}
                disabled={isLoading || tableData.rows.length === 0}
                className="inline-flex items-center gap-2 h-10 px-5 rounded-xl bg-accent text-white text-sm font-semibold hover:bg-accent-hover hover:-translate-y-px active:scale-[0.97] transition disabled:opacity-60"
              >
                <Download size={16} />
                Скачать отчёт (CSV)
              </button>
              <button
                onClick={handleXls}
                disabled={isLoading || tableData.rows.length === 0}
                className="inline-flex items-center gap-2 h-10 px-5 rounded-xl border border-brand-100 bg-white text-sm font-semibold text-ink-900 hover:bg-brand-50 transition disabled:opacity-60"
              >
                <FileSpreadsheet size={16} />
                Скачать XLSX
              </button>
              <button
                onClick={handlePdf}
                disabled={isLoading || tableData.rows.length === 0}
                className="inline-flex items-center gap-2 h-10 px-5 rounded-xl border border-brand-100 bg-white text-sm font-semibold text-ink-900 hover:bg-brand-50 transition disabled:opacity-60"
              >
                <Printer size={16} />
                Печать / PDF
              </button>
              <button
                onClick={handleEmail}
                className="inline-flex items-center gap-2 h-10 px-4 rounded-xl text-sm font-semibold text-brand-600 hover:bg-brand-50 transition-colors"
              >
                <Mail size={16} />
                Отправить на e-mail
              </button>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Секция 5. История отчётов */}
      <div className="bg-surface rounded-card border border-brand-100/60 shadow-card p-5 lg:p-6">
        <h3 className="text-[17px] leading-6 font-semibold text-ink-900 mb-3">Ранее сформированные</h3>
        <div className="divide-y divide-brand-100/60">
          {historyList.map((h, i) => (
            <motion.div
              key={h.id}
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.22, delay: Math.min(i, 11) * 0.03 }}
              onClick={() => {
                setType(h.type)
                openPreview(h.date)
              }}
              className="flex items-center gap-3 py-3 cursor-pointer hover:bg-brand-50/50 -mx-2 px-2 rounded-xl transition-colors"
            >
              <span className="w-9 h-9 rounded-lg bg-brand-50 text-brand-600 flex items-center justify-center shrink-0">
                <FileText size={17} />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-semibold text-ink-900 truncate">{h.name}</span>
                <span className="block text-xs text-ink-500">{h.author}</span>
              </span>
              <span className="hidden sm:block font-mono-num text-ink-500 shrink-0">
                {format(h.date, 'dd.MM.yyyy')}
              </span>
              <span
                className={cn(
                  'shrink-0 rounded-full px-2 py-0.5 text-[11px] font-semibold uppercase tracking-wide',
                  h.formatLabel === 'PDF' ? 'bg-danger-bg text-danger' : 'bg-success-bg text-success'
                )}
              >
                {h.formatLabel}
              </span>
              <button
                onClick={(e) => {
                  e.stopPropagation()
                  setType(h.type)
                  setPreviewOpen(true)
                  setPreviewStamp(h.date)
                  showToast('Отчёт сохранён')
                }}
                aria-label="Скачать"
                className="shrink-0 w-9 h-9 rounded-xl text-brand-600 hover:bg-brand-50 flex items-center justify-center transition-colors"
              >
                <Download size={16} />
              </button>
            </motion.div>
          ))}
        </div>
        <div className="mt-4 rounded-xl bg-brand-50 px-4 py-3 text-[13px] text-ink-500">
          Сформированные отчёты будут храниться здесь 90 дней
        </div>
      </div>

      <Toast text={toast} />
    </div>
  )
}
