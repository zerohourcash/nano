import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { AnimatePresence, animate, motion } from 'framer-motion'
import {
  ArrowLeft,
  Building2,
  Check,
  CheckCircle2,
  ClipboardCheck,
  FileDown,
  Loader2,
  Plus,
  ScanLine,
  TriangleAlert,
  Warehouse,
  X,
} from 'lucide-react'
import { format } from 'date-fns'
import type { inferRouterOutputs } from '@trpc/server'
import type { AppRouter } from '../../api/router'
import { trpc } from '@/providers/trpc'
import { cn } from '@/lib/utils'

type RouterOutputs = inferRouterOutputs<AppRouter>
type SessionListItem = RouterOutputs['inventory']['sessions'][number]
type SessionDetail = NonNullable<RouterOutputs['inventory']['byId']>
type ResultRow = SessionDetail['results'][number]

const EASE = [0.22, 1, 0.36, 1] as [number, number, number, number]

// ─── Статистика и классификация строк ────────────────────────────────────────

interface InvStats {
  matched: number
  surplus: number
  shortage: number
  pending: number
}

type QtyLike = { checked: boolean; expectedQty: number | null; actualQty: number | null }

function computeStats(results: QtyLike[]): InvStats {
  const s: InvStats = { matched: 0, surplus: 0, shortage: 0, pending: 0 }
  for (const r of results) {
    if (!r.checked) {
      s.pending += 1
      continue
    }
    const exp = r.expectedQty ?? 1
    const act = r.actualQty ?? exp
    if (act === exp) s.matched += 1
    else if (act > exp) s.surplus += 1
    else s.shortage += 1
  }
  return s
}

type RowKind = 'pending' | 'matched' | 'shortage' | 'surplus'

function rowKind(r: QtyLike): RowKind {
  if (!r.checked) return 'pending'
  const exp = r.expectedQty ?? 1
  const act = r.actualQty ?? exp
  if (act === exp) return 'matched'
  return act > exp ? 'surplus' : 'shortage'
}

const KIND_BADGE: Record<RowKind, { label: string; bg: string; color: string }> = {
  pending: { label: 'Не проверено', bg: '#ECECF3', color: '#6B6E9E' },
  matched: { label: 'Найдено ✓', bg: '#C8FCD2', color: '#2E9E5B' },
  shortage: { label: 'Недостача', bg: '#FAD8D1', color: '#D64545' },
  surplus: { label: 'Излишек', bg: '#FBFCC8', color: '#A87C0F' },
}

// ─── Тост ────────────────────────────────────────────────────────────────────

interface ToastData {
  text: string
  kind: 'ok' | 'error'
}

function useToast() {
  const [toast, setToast] = useState<ToastData | null>(null)
  useEffect(() => {
    if (!toast) return
    const t = setTimeout(() => setToast(null), 4000)
    return () => clearTimeout(t)
  }, [toast])
  const show = useCallback((text: string, kind: 'ok' | 'error' = 'ok') => setToast({ text, kind }), [])
  return { toast, show }
}

function ToastView({ toast }: { toast: ToastData | null }) {
  return (
    <AnimatePresence>
      {toast && (
        <motion.div
          initial={{ opacity: 0, y: 24 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 12 }}
          transition={{ duration: 0.24 }}
          className="fixed bottom-24 lg:bottom-8 left-1/2 -translate-x-1/2 z-[80] inline-flex max-w-[92vw] items-center gap-2 rounded-full bg-ink-900 text-white text-sm font-semibold px-5 py-3 shadow-modal"
        >
          {toast.kind === 'ok' ? (
            <CheckCircle2 size={16} className="text-teal shrink-0" />
          ) : (
            <TriangleAlert size={16} className="text-danger shrink-0" />
          )}
          <span className="truncate">{toast.text}</span>
        </motion.div>
      )}
    </AnimatePresence>
  )
}

// ─── Прогресс-бар (анимируется при появлении во вьюпорте) ───────────────────

function ProgressBar({ value, total, big }: { value: number; total: number; big?: boolean }) {
  const pct = total > 0 ? Math.min(100, Math.round((value / total) * 100)) : 0
  return (
    <div className="flex items-center gap-3">
      <div
        className={cn(
          'flex-1 rounded-full bg-brand-100/70 overflow-hidden',
          big ? 'h-2.5' : 'h-2'
        )}
      >
        <motion.div
          className="h-full rounded-full bg-teal"
          initial={{ width: 0 }}
          whileInView={{ width: `${pct}%` }}
          viewport={{ once: true }}
          transition={{ duration: 0.8, ease: EASE }}
        />
      </div>
      <span className="font-mono-num text-ink-500 shrink-0">
        {value}/{total} ед.
      </span>
    </div>
  )
}

// ─── Страница ────────────────────────────────────────────────────────────────

export default function Inventory() {
  const [activeId, setActiveId] = useState<number | null>(null)
  const [createOpen, setCreateOpen] = useState(false)
  const { toast, show } = useToast()

  const sessionsQ = trpc.inventory.sessions.useQuery()
  const sessions = useMemo(() => sessionsQ.data ?? [], [sessionsQ.data])

  if (activeId != null) {
    return (
      <>
        <SessionView sessionId={activeId} onBack={() => setActiveId(null)} showToast={show} />
        <ToastView toast={toast} />
      </>
    )
  }

  const nextNumber = `ИНВ-${String(sessions.length + 1).padStart(3, '0')}`

  return (
    <div className="space-y-4 lg:space-y-6">
      {/* ── Заголовок ── */}
      <motion.div
        initial={{ opacity: 0, y: 16 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.28, ease: EASE }}
        className="flex flex-wrap items-center gap-3"
      >
        <div>
          <h1 className="text-2xl lg:text-[28px] leading-9 font-bold tracking-[-0.01em] text-ink-900">
            Инвентаризация
          </h1>
          <p className="mt-1 text-[13px] leading-[18px] text-ink-500">
            Сверка наличия по складам и объектам
          </p>
        </div>
        <button
          type="button"
          onClick={() => setCreateOpen(true)}
          className="ml-auto inline-flex h-10 items-center gap-2 rounded-xl bg-accent px-5 text-sm font-semibold text-white transition-all hover:bg-accent-hover hover:-translate-y-px active:scale-[0.97]"
        >
          <Plus size={16} />
          Новая инвентаризация
        </button>
      </motion.div>

      {/* ── Список сессий ── */}
      {sessionsQ.isLoading ? (
        <div className="space-y-4">
          {Array.from({ length: 3 }).map((_, i) => (
            <div
              key={i}
              className="h-[132px] rounded-card border border-brand-100/60 bg-surface shadow-card animate-skeleton-pulse"
            />
          ))}
        </div>
      ) : sessionsQ.isError ? (
        <div className="rounded-card border border-brand-100/60 bg-surface p-6 shadow-card text-sm text-danger">
          Не удалось загрузить сессии инвентаризации. Попробуйте обновить страницу.
        </div>
      ) : sessions.length === 0 ? (
        <motion.div
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.28 }}
          className="rounded-card border border-brand-100/60 bg-surface shadow-card px-6 py-12 flex flex-col items-center text-center"
        >
          <img
            src="/empty-catalog.svg"
            alt="Нет инвентаризаций"
            className="w-[240px] max-w-full h-auto"
          />
          <h3 className="mt-6 text-[17px] leading-6 font-semibold text-ink-900">
            Инвентаризаций пока не было
          </h3>
          <p className="mt-2 max-w-sm text-[13px] leading-[18px] text-ink-500">
            Запустите первую сверку — отметьте наличие по QR или вручную
          </p>
          <button
            type="button"
            onClick={() => setCreateOpen(true)}
            className="mt-6 inline-flex h-10 items-center gap-2 rounded-xl bg-accent px-5 text-sm font-semibold text-white transition-all hover:bg-accent-hover hover:-translate-y-px active:scale-[0.97]"
          >
            <Plus size={16} />
            Новая инвентаризация
          </button>
        </motion.div>
      ) : (
        <div className="space-y-4">
          {sessions.map((s, i) => (
            <SessionCard key={s.id} s={s} index={i} onOpen={() => setActiveId(s.id)} />
          ))}
        </div>
      )}

      <CreateSessionModal
        open={createOpen}
        nextNumber={nextNumber}
        onClose={() => setCreateOpen(false)}
        onCreated={(id) => setActiveId(id)}
        showToast={show}
      />
      <ToastView toast={toast} />
    </div>
  )
}

// ─── Карточка сессии ─────────────────────────────────────────────────────────

function SessionCard({
  s,
  index,
  onOpen,
}: {
  s: SessionListItem
  index: number
  onOpen: () => void
}) {
  const inProgress = s.status === 'in_progress'
  const created = new Date(s.createdAt)
  const dateRange = s.completedAt
    ? `${format(created, 'dd.MM')}–${format(new Date(s.completedAt), 'dd.MM.yyyy')}`
    : `начата ${format(created, 'dd.MM.yyyy')}`

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.26, delay: Math.min(index, 12) * 0.06, ease: EASE }}
      onClick={onOpen}
      className="cursor-pointer rounded-card border border-brand-100/60 bg-surface p-4 lg:p-5 shadow-card transition-all duration-200 hover:shadow-hover hover:-translate-y-0.5"
    >
      <div className="flex flex-wrap items-center gap-3">
        <span className="font-mono text-[15px] font-semibold text-ink-900">{s.number}</span>
        {inProgress ? (
          <span className="inline-flex items-center gap-1.5 rounded-full bg-warning-bg px-2.5 py-0.5 text-caption text-warning">
            <span className="relative flex h-1.5 w-1.5">
              <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-warning opacity-75" />
              <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-warning" />
            </span>
            В процессе
          </span>
        ) : (
          <span className="inline-flex items-center gap-1 rounded-full bg-success-bg px-2.5 py-0.5 text-caption text-success">
            <Check size={12} strokeWidth={3} />
            Завершена
          </span>
        )}
        <span className="ml-auto text-sm font-semibold text-brand-600">
          {inProgress ? 'Продолжить →' : 'Открыть отчёт →'}
        </span>
      </div>

      <div className="mt-1.5 text-[13px] leading-[18px] text-ink-500">
        {dateRange}
        {s.starter?.fullName ? ` · ${s.starter.fullName}` : ''}
      </div>

      <div className="mt-3">
        <ProgressBar value={s.checkedItems} total={s.totalItems} />
      </div>

      {!inProgress && <SessionStatsLine sessionId={s.id} />}
    </motion.div>
  )
}

/** Мини-статы завершённой сессии (ленивый запрос результатов) */
function SessionStatsLine({ sessionId }: { sessionId: number }) {
  const resultsQ = trpc.inventory.results.useQuery({ sessionId })
  if (!resultsQ.data) {
    return <div className="mt-3 h-5 w-64 rounded bg-brand-50 animate-skeleton-pulse" />
  }
  const stats = computeStats(resultsQ.data)
  return (
    <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-[13px] leading-[18px] font-semibold">
      <span className="text-success">Совпало {stats.matched}</span>
      <span className="text-brand-600">Излишки {stats.surplus}</span>
      <span className="text-danger">Недостача {stats.shortage}</span>
    </div>
  )
}

// ─── Модалка «Новая инвентаризация» ─────────────────────────────────────────

type ScopeKey = 'all' | 'storage' | 'site'

const SCOPES: { key: ScopeKey; label: string }[] = [
  { key: 'all', label: 'Всё пространство' },
  { key: 'storage', label: 'Склад' },
  { key: 'site', label: 'Объект' },
]

function CreateSessionModal({
  open,
  nextNumber,
  onClose,
  onCreated,
  showToast,
}: {
  open: boolean
  nextNumber: string
  onClose: () => void
  onCreated: (id: number) => void
  showToast: (text: string, kind?: 'ok' | 'error') => void
}) {
  const utils = trpc.useUtils()
  const [scope, setScope] = useState<ScopeKey>('all')
  const [storageId, setStorageId] = useState<number | null>(null)
  const [blockTransfers, setBlockTransfers] = useState(false)

  const meQ = trpc.meta.currentUser.useQuery(undefined, { enabled: open })
  const storagesQ = trpc.admin.storages.list.useQuery(undefined, {
    enabled: open && scope === 'storage',
  })

  const create = trpc.inventory.create.useMutation({
    onSuccess: (session) => {
      utils.inventory.sessions.invalidate()
      onClose()
      if (session) {
        showToast(`Инвентаризация ${session.number} начата`)
        onCreated(session.id)
      }
    },
    onError: (e) => showToast(e.message, 'error'),
  })

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, onClose])

  const submit = () => {
    if (scope === 'storage' && storageId) create.mutate({ storageId })
    else create.mutate({})
  }

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.18 }}
          className="fixed inset-0 z-[70] flex items-end sm:items-center justify-center bg-[#303466]/45 backdrop-blur-sm p-0 sm:p-4"
          onClick={onClose}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 12 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.97, y: 8 }}
            transition={{ type: 'spring', stiffness: 300, damping: 26 }}
            onClick={(e) => e.stopPropagation()}
            className="w-full max-w-[560px] rounded-t-modal sm:rounded-modal bg-surface shadow-modal p-5 lg:p-6 max-h-[92dvh] overflow-y-auto"
          >
            <div className="flex items-center justify-between">
              <h3 className="text-[17px] leading-6 font-semibold text-ink-900">
                Новая инвентаризация
              </h3>
              <button
                type="button"
                onClick={onClose}
                className="flex h-10 w-10 items-center justify-center rounded-xl text-ink-500 transition-colors hover:bg-brand-50"
                aria-label="Закрыть"
              >
                <X size={18} />
              </button>
            </div>

            <div className="mt-5 space-y-4">
              {/* Номер */}
              <div>
                <label className="text-[13px] leading-[18px] font-semibold text-ink-500">
                  Номер сессии
                </label>
                <div className="mt-1 flex h-11 items-center rounded-xl border border-brand-100 bg-brand-50/60 px-4">
                  <span className="font-mono text-sm font-semibold text-ink-900">{nextNumber}</span>
                  <span className="ml-auto text-xs text-ink-300">присваивается автоматически</span>
                </div>
              </div>

              {/* Область сверки */}
              <div>
                <label className="text-[13px] leading-[18px] font-semibold text-ink-500">
                  Область сверки
                </label>
                <div className="mt-1 flex rounded-xl border border-brand-100 bg-surface p-1">
                  {SCOPES.map((sc) => {
                    const active = scope === sc.key
                    return (
                      <button
                        key={sc.key}
                        type="button"
                        onClick={() => setScope(sc.key)}
                        className={cn(
                          'relative flex-1 h-9 rounded-lg text-sm font-semibold transition-colors',
                          active ? 'text-white' : 'text-ink-500 hover:text-ink-900'
                        )}
                      >
                        {active && (
                          <motion.span
                            layoutId="inv-scope-seg"
                            className="absolute inset-0 rounded-lg bg-brand-600"
                            transition={{ duration: 0.2, ease: EASE }}
                          />
                        )}
                        <span className="relative z-10">{sc.label}</span>
                      </button>
                    )
                  })}
                </div>

                {scope === 'storage' && (
                  <motion.div
                    initial={{ opacity: 0, height: 0 }}
                    animate={{ opacity: 1, height: 'auto' }}
                    transition={{ duration: 0.2 }}
                    className="overflow-hidden"
                  >
                    <select
                      value={storageId ?? ''}
                      onChange={(e) => setStorageId(e.target.value ? Number(e.target.value) : null)}
                      className="mt-2 h-11 w-full rounded-xl border border-brand-100 bg-surface px-4 text-sm text-ink-900 focus:border-brand-600 focus:ring-[3px] focus:ring-[#5E629B22]"
                    >
                      <option value="" disabled>
                        {storagesQ.isLoading ? 'Загрузка складов…' : 'Выберите склад'}
                      </option>
                      {(storagesQ.data ?? []).map((st) => (
                        <option key={st.id} value={st.id}>
                          {st.name}
                        </option>
                      ))}
                    </select>
                  </motion.div>
                )}

                {scope === 'site' && (
                  <div className="mt-2 flex items-start gap-2 rounded-xl bg-info-bg border-l-[3px] border-teal px-3 py-2.5 text-sm text-ink-900">
                    <ClipboardCheck size={16} className="mt-0.5 shrink-0 text-teal-dark" />
                    В демо-версии сверка по объекту охватывает все позиции пространства.
                  </div>
                )}
              </div>

              {/* Ответственный */}
              <div>
                <label className="text-[13px] leading-[18px] font-semibold text-ink-500">
                  Ответственный за сверку
                </label>
                <div className="mt-1 flex h-11 items-center gap-2 rounded-xl border border-brand-100 bg-brand-50/60 px-4 text-sm text-ink-900">
                  <span className="font-semibold">{meQ.data?.fullName ?? '…'}</span>
                  <span className="text-xs text-ink-300">(вы)</span>
                </div>
              </div>

              {/* Блокировка передач */}
              <label className="flex cursor-pointer items-start gap-3">
                <input
                  type="checkbox"
                  checked={blockTransfers}
                  onChange={(e) => setBlockTransfers(e.target.checked)}
                  className="peer sr-only"
                />
                <span
                  className={cn(
                    'mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-md border-[1.5px] transition-colors',
                    blockTransfers ? 'border-brand-600 bg-brand-600' : 'border-brand-100 bg-surface'
                  )}
                >
                  {blockTransfers && <Check size={13} strokeWidth={3} className="text-white" />}
                </span>
                <span className="text-sm leading-5 text-ink-900">
                  Блокировать передачи в области сверки до завершения
                  {blockTransfers && (
                    <span className="block text-xs text-ink-300">
                      В демо-версии блокировка не применяется
                    </span>
                  )}
                </span>
              </label>
            </div>

            <div className="mt-6 flex justify-end gap-2">
              <button
                type="button"
                onClick={onClose}
                className="h-10 rounded-xl border border-brand-100 bg-surface px-5 text-sm font-semibold text-ink-900 transition-colors hover:bg-brand-50"
              >
                Отмена
              </button>
              <button
                type="button"
                onClick={submit}
                disabled={create.isPending || (scope === 'storage' && !storageId)}
                className="inline-flex h-10 items-center gap-2 rounded-xl bg-accent px-5 text-sm font-semibold text-white transition-all hover:bg-accent-hover active:scale-[0.97] disabled:opacity-50"
              >
                {create.isPending && <Loader2 size={16} className="animate-spin" />}
                Начать сверку
              </button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}

// ─── Режим сверки / отчёт по сессии ─────────────────────────────────────────

function SessionView({
  sessionId,
  onBack,
  showToast,
}: {
  sessionId: number
  onBack: () => void
  showToast: (text: string, kind?: 'ok' | 'error') => void
}) {
  const utils = trpc.useUtils()
  const detailQ = trpc.inventory.byId.useQuery({ id: sessionId })
  const storagesQ = trpc.admin.storages.list.useQuery()
  const sitesQ = trpc.admin.buildingSites.list.useQuery()

  const [manualId, setManualId] = useState('')
  const [scanTarget, setScanTarget] = useState<ResultRow | null>(null)
  const [confirmOpen, setConfirmOpen] = useState(false)
  const [summaryOpen, setSummaryOpen] = useState(false)

  const session = detailQ.data ?? null
  const inProgress = session?.status === 'in_progress'

  const stats = useMemo(() => computeStats(session?.results ?? []), [session])

  const sortedResults = useMemo(() => {
    const results = [...(session?.results ?? [])]
    if (inProgress) {
      // Непроверенные сверху, отмеченные «утонут» внизу (FLIP через layout)
      return results.sort((a, b) => Number(a.checked) - Number(b.checked))
    }
    const order: Record<RowKind, number> = { shortage: 0, surplus: 1, matched: 2, pending: 3 }
    return results.sort((a, b) => order[rowKind(a)] - order[rowKind(b)])
  }, [session, inProgress])

  const invalidate = useCallback(() => {
    utils.inventory.byId.invalidate({ id: sessionId })
    utils.inventory.sessions.invalidate()
    utils.inventory.results.invalidate({ sessionId })
  }, [utils, sessionId])

  const checkItem = trpc.inventory.checkItem.useMutation({
    onSuccess: invalidate,
    onError: (e) => showToast(e.message, 'error'),
  })
  const complete = trpc.inventory.complete.useMutation({
    onSuccess: () => {
      invalidate()
      setSummaryOpen(true)
    },
    onError: (e) => showToast(e.message, 'error'),
  })

  const placeLabel = useCallback(
    (item: ResultRow['item']) => {
      if (item?.buildingSiteId) {
        return sitesQ.data?.find((x) => x.id === item.buildingSiteId)?.name ?? 'Объект'
      }
      if (item?.storageId) {
        return storagesQ.data?.find((x) => x.id === item.storageId)?.name ?? 'Склад'
      }
      return 'Место не указано'
    },
    [sitesQ.data, storagesQ.data]
  )

  const doCheck = (r: ResultRow, qty?: number) => {
    checkItem.mutate(
      { sessionId, itemId: r.itemId, ...(qty !== undefined ? { actualQty: qty } : {}) },
      { onSuccess: () => showToast(`Отмечено: ${r.item?.title ?? `позиция #${r.itemId}`}`) }
    )
  }

  const doMissing = (r: ResultRow) => {
    checkItem.mutate(
      { sessionId, itemId: r.itemId, actualQty: 0 },
      { onSuccess: () => showToast(`Недостача: ${r.item?.title ?? `позиция #${r.itemId}`}`) }
    )
  }

  const doUncheck = (r: ResultRow) => {
    checkItem.mutate({ sessionId, itemId: r.itemId, checked: false })
  }

  const openScanner = () => {
    const target = sortedResults.find((r) => !r.checked)
    if (!target) {
      showToast('Все позиции уже проверены')
      return
    }
    setScanTarget(target)
  }

  const handleScanDetected = () => {
    if (scanTarget) doCheck(scanTarget, scanTarget.expectedQty ?? undefined)
    setScanTarget(null)
  }

  const submitManual = (e: React.FormEvent) => {
    e.preventDefault()
    const raw = manualId.trim()
    const q = raw.toLowerCase()
    if (!q) return
    const match = sortedResults.find((r) => {
      const iid = (r.item?.internalId ?? '').toLowerCase()
      return iid === q || iid === `вн-${q}`
    })
    if (!match) {
      showToast(`Позиция «${raw}» не входит в эту сверку`, 'error')
      return
    }
    if (match.checked) {
      showToast('Эта позиция уже отмечена')
      return
    }
    doCheck(match, match.expectedQty ?? undefined)
    setManualId('')
  }

  const requestComplete = () => {
    if (stats.pending > 0) setConfirmOpen(true)
    else complete.mutate({ sessionId })
  }

  if (detailQ.isLoading) {
    return (
      <div className="space-y-4">
        <div className="h-10 w-48 rounded-xl bg-surface animate-skeleton-pulse" />
        <div className="h-28 rounded-card border border-brand-100/60 bg-surface shadow-card animate-skeleton-pulse" />
        {Array.from({ length: 4 }).map((_, i) => (
          <div
            key={i}
            className="h-[84px] rounded-card border border-brand-100/60 bg-surface shadow-card animate-skeleton-pulse"
          />
        ))}
      </div>
    )
  }

  if (detailQ.isError || !session) {
    return (
      <div className="space-y-4">
        <button
          type="button"
          onClick={onBack}
          className="inline-flex h-10 items-center gap-2 rounded-xl px-4 text-sm font-semibold text-brand-600 transition-colors hover:bg-brand-50"
        >
          <ArrowLeft size={16} />
          Все сессии
        </button>
        <div className="rounded-card border border-brand-100/60 bg-surface p-6 shadow-card text-sm text-danger">
          Сессия не найдена или не удалось её загрузить.
        </div>
      </div>
    )
  }

  const total = session.results.length
  const checked = stats.matched + stats.surplus + stats.shortage

  return (
    <div className="space-y-4 lg:space-y-6 pb-20 lg:pb-0">
      {/* ── Шапка сверки ── */}
      <motion.div
        initial={{ opacity: 0, y: 16 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.28, ease: EASE }}
        className="space-y-3"
      >
        <button
          type="button"
          onClick={onBack}
          className="inline-flex h-9 items-center gap-2 rounded-xl px-3 text-sm font-semibold text-brand-600 transition-colors hover:bg-brand-50"
        >
          <ArrowLeft size={16} />
          Все сессии
        </button>

        <div className="rounded-card border border-brand-100/60 bg-surface p-4 lg:p-5 shadow-card">
          <div className="flex flex-wrap items-center gap-3">
            <h1 className="font-mono text-xl lg:text-2xl font-semibold text-ink-900">
              {session.number}
            </h1>
            {inProgress ? (
              <span className="inline-flex items-center gap-1.5 rounded-full bg-warning-bg px-2.5 py-0.5 text-caption text-warning">
                <span className="relative flex h-1.5 w-1.5">
                  <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-warning opacity-75" />
                  <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-warning" />
                </span>
                В процессе
              </span>
            ) : (
              <span className="inline-flex items-center gap-1 rounded-full bg-success-bg px-2.5 py-0.5 text-caption text-success">
                <Check size={12} strokeWidth={3} />
                Завершена
              </span>
            )}
            <div className="ml-auto">
              {inProgress ? (
                <button
                  type="button"
                  onClick={requestComplete}
                  disabled={complete.isPending}
                  className="inline-flex h-10 items-center gap-2 rounded-xl bg-accent px-5 text-sm font-semibold text-white transition-all hover:bg-accent-hover hover:-translate-y-px active:scale-[0.97] disabled:opacity-50"
                >
                  {complete.isPending && <Loader2 size={16} className="animate-spin" />}
                  Завершить сверку
                </button>
              ) : (
                <button
                  type="button"
                  onClick={() => showToast(`Акт инвентаризации ${session.number} сформирован (PDF, демо)`)}
                  className="inline-flex h-10 items-center gap-2 rounded-xl border border-brand-100 bg-surface px-5 text-sm font-semibold text-ink-900 transition-colors hover:bg-brand-50"
                >
                  <FileDown size={16} />
                  Сформировать акт
                </button>
              )}
            </div>
          </div>

          <div className="mt-1 text-[13px] leading-[18px] text-ink-500">
            {format(new Date(session.createdAt), 'dd.MM.yyyy')}
            {session.completedAt ? ` — ${format(new Date(session.completedAt), 'dd.MM.yyyy')}` : ''}
            {session.starter?.fullName ? ` · ${session.starter.fullName}` : ''}
          </div>

          <div className="mt-3">
            <div className="mb-1.5 text-sm font-semibold text-ink-900">
              Проверено {checked} из {total}
            </div>
            <ProgressBar value={checked} total={total} big />
          </div>

          {!inProgress && (
            <div className="mt-4 flex flex-wrap gap-x-6 gap-y-1 text-sm font-semibold">
              <span className="text-success">Совпало: {stats.matched}</span>
              <span className="text-brand-600">Излишки: {stats.surplus}</span>
              <span className="text-danger">Недостача: {stats.shortage}</span>
              {stats.pending > 0 && (
                <span className="text-ink-300">Без отметки: {stats.pending}</span>
              )}
            </div>
          )}
        </div>
      </motion.div>

      {/* ── Сканер-панель ── */}
      {inProgress && (
        <motion.div
          initial={{ opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.28, delay: 0.05, ease: EASE }}
          className="rounded-card border-2 border-teal bg-[#D8F2F0]/50 p-4 lg:p-5"
        >
          <div className="flex items-center gap-2 text-teal-dark">
            <ScanLine size={20} strokeWidth={1.75} />
            <h2 className="text-[15px] leading-[22px] font-semibold">Сканер QR</h2>
            <span className="text-[13px] text-teal-dark/70">
              Отсканируйте код на инструменте или введите вн. номер вручную
            </span>
          </div>
          <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center">
            <button
              type="button"
              onClick={openScanner}
              className="hidden lg:inline-flex h-12 items-center gap-2.5 rounded-xl bg-accent px-6 text-[15px] font-semibold text-white transition-all hover:bg-accent-hover hover:-translate-y-px active:scale-[0.97]"
            >
              <ScanLine size={24} />
              Сканировать QR
            </button>
            <form onSubmit={submitManual} className="flex flex-1 gap-2">
              <input
                value={manualId}
                onChange={(e) => setManualId(e.target.value)}
                placeholder="ВН-0142"
                className="h-11 flex-1 rounded-xl border border-brand-100 bg-surface px-4 font-mono text-sm font-semibold uppercase text-ink-900 placeholder:text-ink-300 focus:border-brand-600 focus:ring-[3px] focus:ring-[#5E629B22]"
              />
              <button
                type="submit"
                disabled={checkItem.isPending}
                className="h-11 rounded-xl border border-brand-100 bg-surface px-5 text-sm font-semibold text-ink-900 transition-colors hover:bg-brand-50 disabled:opacity-50"
              >
                Отметить
              </button>
            </form>
          </div>
        </motion.div>
      )}

      {/* ── Список сверки ── */}
      {total === 0 ? (
        <div className="rounded-card border border-brand-100/60 bg-surface p-8 shadow-card text-center text-sm text-ink-500">
          В области сверки нет позиций
        </div>
      ) : (
        <ul className="space-y-3">
          {sortedResults.map((r) => (
            <ResultRowView
              key={r.id}
              r={r}
              readOnly={!inProgress}
              busy={checkItem.isPending}
              place={placeLabel(r.item)}
              onCheck={(qty) => doCheck(r, qty)}
              onMissing={() => doMissing(r)}
              onUncheck={() => doUncheck(r)}
            />
          ))}
        </ul>
      )}

      {/* Мобайл: большая sticky-кнопка сканирования */}
      {inProgress && (
        <div className="lg:hidden fixed bottom-24 left-3 right-3 z-40">
          <button
            type="button"
            onClick={openScanner}
            className="flex h-12 w-full items-center justify-center gap-2.5 rounded-xl bg-accent text-[15px] font-semibold text-white shadow-hover transition-all active:scale-[0.97]"
          >
            <ScanLine size={22} />
            Сканировать QR
          </button>
        </div>
      )}

      {/* ── Оверлеи ── */}
      <AnimatePresence>
        {scanTarget && (
          <ScannerOverlay
            key="scanner"
            itemTitle={scanTarget.item?.title ?? ''}
            itemInternalId={scanTarget.item?.internalId ?? ''}
            onCancel={() => setScanTarget(null)}
            onDetected={handleScanDetected}
          />
        )}
      </AnimatePresence>

      <ConfirmCompleteModal
        open={confirmOpen}
        pending={stats.pending}
        busy={complete.isPending}
        onCancel={() => setConfirmOpen(false)}
        onConfirm={() => {
          setConfirmOpen(false)
          complete.mutate({ sessionId })
        }}
      />

      <SummaryModal
        open={summaryOpen}
        number={session.number}
        stats={stats}
        onAct={() => {
          setSummaryOpen(false)
          showToast(`Акт инвентаризации ${session.number} сформирован (PDF, демо)`)
        }}
        onBack={() => {
          setSummaryOpen(false)
          onBack()
        }}
      />
    </div>
  )
}

// ─── Строка сверки ───────────────────────────────────────────────────────────

function ResultRowView({
  r,
  readOnly,
  busy,
  place,
  onCheck,
  onMissing,
  onUncheck,
}: {
  r: ResultRow
  readOnly: boolean
  busy: boolean
  place: string
  onCheck: (qty?: number) => void
  onMissing: () => void
  onUncheck: () => void
}) {
  const kind = rowKind(r)
  const badge = KIND_BADGE[kind]
  const item = r.item
  const photo = item?.photos.find((p) => p.isTitle) ?? item?.photos[0]
  const quantitative = Boolean(item?.quantitative)
  const [qty, setQty] = useState<string>(r.expectedQty != null ? String(r.expectedQty) : '')

  return (
    <motion.li
      layout="position"
      transition={{ duration: 0.3, ease: EASE }}
      className={cn(
        'flex flex-wrap items-center gap-3 rounded-card border p-3 sm:p-4 shadow-card transition-colors duration-300',
        kind === 'matched' && 'border-success/30 bg-success-bg/60',
        kind === 'shortage' && 'border-danger/30 bg-danger-bg/50',
        kind === 'surplus' && 'border-warning/40 bg-warning-bg/50',
        kind === 'pending' && 'border-brand-100/60 bg-surface'
      )}
    >
      {/* Статусная иконка */}
      <span
        className={cn(
          'flex h-8 w-8 shrink-0 items-center justify-center rounded-full border-2',
          kind === 'pending' && 'border-dashed border-brand-100 text-ink-300',
          kind === 'matched' && 'border-success bg-success text-white',
          kind === 'shortage' && 'border-danger bg-danger text-white',
          kind === 'surplus' && 'border-warning bg-warning text-white'
        )}
      >
        {kind === 'matched' && (
          <motion.svg
            viewBox="0 0 24 24"
            className="h-4 w-4"
            initial={false}
          >
            <motion.path
              d="M5 12.5l4.5 4.5L19 7.5"
              fill="none"
              stroke="currentColor"
              strokeWidth={3}
              strokeLinecap="round"
              strokeLinejoin="round"
              initial={{ pathLength: 0 }}
              animate={{ pathLength: 1 }}
              transition={{ duration: 0.2 }}
            />
          </motion.svg>
        )}
        {kind === 'shortage' && <X size={14} strokeWidth={3} />}
        {kind === 'surplus' && <TriangleAlert size={13} strokeWidth={2.5} />}
      </span>

      {/* Фото */}
      {photo?.url ? (
        <img
          src={photo.url}
          alt={item?.title ?? ''}
          className="h-14 w-14 shrink-0 rounded-lg bg-brand-50 object-cover"
        />
      ) : (
        <span className="flex h-14 w-14 shrink-0 items-center justify-center rounded-lg bg-brand-50 text-ink-300">
          <ClipboardCheck size={20} />
        </span>
      )}

      {/* Основное */}
      <div className="min-w-0 flex-1 basis-40">
        <div className="font-mono-num text-ink-500">{item?.internalId ?? `#${r.itemId}`}</div>
        <div className="truncate text-[15px] leading-[22px] font-semibold text-ink-900">
          {item?.title ?? 'Позиция'}
        </div>
        <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[13px] leading-[18px] text-ink-500">
          {item?.category?.name && <span>{item.category.name}</span>}
          <span className="inline-flex items-center gap-1">
            {item?.buildingSiteId ? <Building2 size={12} /> : <Warehouse size={12} />}
            {place}
          </span>
          {quantitative && (
            <span className="font-mono-num">
              Ожидается: {r.expectedQty ?? '—'} {item?.unit ?? 'шт'}
            </span>
          )}
        </div>
      </div>

      {/* Бейдж + действия */}
      <div className="flex flex-wrap items-center gap-2">
        <span
          className="inline-flex items-center rounded-full px-2.5 py-0.5 text-caption"
          style={{ background: badge.bg, color: badge.color }}
        >
          {badge.label}
        </span>

        {!readOnly && !r.checked && (
          <>
            {quantitative && (
              <input
                type="number"
                min={0}
                value={qty}
                onChange={(e) => setQty(e.target.value)}
                placeholder="Факт"
                className="h-9 w-20 rounded-lg border border-brand-100 bg-surface px-2 font-mono text-sm font-semibold text-ink-900 focus:border-brand-600"
              />
            )}
            <button
              type="button"
              disabled={busy}
              onClick={() => {
                const q = quantitative && qty !== '' ? Number(qty) : (r.expectedQty ?? undefined)
                onCheck(q)
              }}
              className="h-9 rounded-lg bg-accent px-4 text-[13px] font-semibold text-white transition-all hover:bg-accent-hover active:scale-[0.97] disabled:opacity-50"
            >
              Отметить
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={onMissing}
              className="h-9 rounded-lg border border-danger/40 px-4 text-[13px] font-semibold text-danger transition-colors hover:bg-danger-bg disabled:opacity-50"
            >
              Не на месте
            </button>
          </>
        )}

        {!readOnly && r.checked && (
          <button
            type="button"
            disabled={busy}
            onClick={onUncheck}
            className="h-9 rounded-lg px-3 text-[13px] font-semibold text-brand-600 transition-colors hover:bg-brand-50 disabled:opacity-50"
          >
            Снять отметку
          </button>
        )}
      </div>
    </motion.li>
  )
}

// ─── Сканер-оверлей (визуальный макет камеры) ───────────────────────────────

function ScannerOverlay({
  itemTitle,
  itemInternalId,
  onCancel,
  onDetected,
}: {
  itemTitle: string
  itemInternalId: string
  onCancel: () => void
  onDetected: () => void
}) {
  const [found, setFound] = useState(false)
  const detectedRef = useRef(onDetected)

  useEffect(() => {
    detectedRef.current = onDetected
  }, [onDetected])

  useEffect(() => {
    const t1 = setTimeout(() => setFound(true), 2200)
    const t2 = setTimeout(() => detectedRef.current(), 2800)
    return () => {
      clearTimeout(t1)
      clearTimeout(t2)
    }
  }, [])

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2 }}
      className="fixed inset-0 z-[70] flex flex-col items-center justify-center bg-[#2E3160]/95 backdrop-blur-sm p-4"
    >
      {/* Видоискатель */}
      <motion.div
        animate={
          found
            ? { borderColor: '#2E9E5B', boxShadow: '0 0 0 4px #2E9E5B55, 0 0 32px #2E9E5B88' }
            : { borderColor: '#66C6BE', boxShadow: '0 0 0 1px #66C6BE33' }
        }
        transition={{ duration: 0.25 }}
        className="relative h-[280px] w-[320px] max-w-[88vw] overflow-hidden rounded-2xl border-2 bg-[#23264D]"
      >
        {/* Уголки рамки */}
        {[
          'left-3 top-3 border-l-[3px] border-t-[3px] rounded-tl-lg',
          'right-3 top-3 border-r-[3px] border-t-[3px] rounded-tr-lg',
          'left-3 bottom-3 border-l-[3px] border-b-[3px] rounded-bl-lg',
          'right-3 bottom-3 border-r-[3px] border-b-[3px] rounded-br-lg',
        ].map((cls) => (
          <span
            key={cls}
            className={cn('absolute h-8 w-8 transition-colors duration-200', cls)}
            style={{ borderColor: found ? '#2E9E5B' : '#66C6BE' }}
          />
        ))}

        {/* Декоративные узлы mesh-сети */}
        <span className="absolute left-[18%] top-[26%] h-1.5 w-1.5 rounded-full bg-teal/40" />
        <span className="absolute left-[70%] top-[18%] h-1 w-1 rounded-full bg-brand-100/40" />
        <span className="absolute left-[82%] top-[62%] h-1.5 w-1.5 rounded-full bg-teal/30" />
        <span className="absolute left-[30%] top-[74%] h-1 w-1 rounded-full bg-brand-100/30" />

        {/* Сканирующая линия */}
        {!found && (
          <motion.div
            className="absolute left-4 right-4 top-3 h-0.5 rounded-full"
            style={{
              background: 'linear-gradient(90deg, transparent, #66C6BE, transparent)',
              boxShadow: '0 0 14px #66C6BE, 0 0 4px #66C6BE',
            }}
            animate={{ y: [0, 248] }}
            transition={{ duration: 1.5, repeat: Infinity, repeatType: 'mirror', ease: 'easeInOut' }}
          />
        )}

        {/* Распознано */}
        <AnimatePresence>
          {found && (
            <motion.div
              initial={{ opacity: 0, scale: 0.8 }}
              animate={{ opacity: 1, scale: 1 }}
              transition={{ type: 'spring', stiffness: 300, damping: 22 }}
              className="absolute inset-0 flex flex-col items-center justify-center gap-2 px-6 text-center"
            >
              <span className="flex h-14 w-14 items-center justify-center rounded-full bg-success text-white">
                <Check size={28} strokeWidth={3} />
              </span>
              <div className="font-mono-num text-teal">{itemInternalId}</div>
              <div className="text-sm font-semibold text-white">{itemTitle}</div>
            </motion.div>
          )}
        </AnimatePresence>
      </motion.div>

      <div className="mt-5 text-center text-sm text-white/80">
        {found ? 'Распознано — отмечаем позицию…' : 'Наведите камеру на QR-код инструмента'}
      </div>

      <button
        type="button"
        onClick={onCancel}
        className="mt-6 h-11 rounded-xl border border-white/25 px-6 text-sm font-semibold text-white transition-colors hover:bg-white/10"
      >
        Отмена
      </button>
    </motion.div>
  )
}

// ─── Подтверждение завершения с непроверенными ──────────────────────────────

function ConfirmCompleteModal({
  open,
  pending,
  busy,
  onCancel,
  onConfirm,
}: {
  open: boolean
  pending: number
  busy: boolean
  onCancel: () => void
  onConfirm: () => void
}) {
  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.18 }}
          className="fixed inset-0 z-[70] flex items-center justify-center bg-[#303466]/45 backdrop-blur-sm p-4"
          onClick={onCancel}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.97 }}
            transition={{ type: 'spring', stiffness: 300, damping: 26 }}
            onClick={(e) => e.stopPropagation()}
            className="w-full max-w-[440px] rounded-modal bg-surface shadow-modal p-6"
          >
            <div className="flex items-start gap-3">
              <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-warning-bg text-warning">
                <TriangleAlert size={18} />
              </span>
              <div>
                <h3 className="text-[17px] leading-6 font-semibold text-ink-900">
                  Завершить с непроверенными?
                </h3>
                <p className="mt-1 text-sm leading-5 text-ink-500">
                  {pending}{' '}
                  {pending === 1 ? 'позиция останется' : pending < 5 ? 'позиции останутся' : 'позиций останутся'}{' '}
                  без отметки. Они не попадут в акт сверки как проверенные.
                </p>
              </div>
            </div>
            <div className="mt-6 flex justify-end gap-2">
              <button
                type="button"
                onClick={onCancel}
                className="h-10 rounded-xl border border-brand-100 bg-surface px-5 text-sm font-semibold text-ink-900 transition-colors hover:bg-brand-50"
              >
                Отмена
              </button>
              <button
                type="button"
                onClick={onConfirm}
                disabled={busy}
                className="inline-flex h-10 items-center gap-2 rounded-xl bg-accent px-5 text-sm font-semibold text-white transition-all hover:bg-accent-hover active:scale-[0.97] disabled:opacity-50"
              >
                {busy && <Loader2 size={16} className="animate-spin" />}
                Завершить
              </button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}

// ─── Модалка итогов ──────────────────────────────────────────────────────────

function CountUp({ value, className }: { value: number; className?: string }) {
  const [v, setV] = useState(0)
  useEffect(() => {
    const controls = animate(0, value, {
      duration: 0.9,
      ease: EASE,
      onUpdate: (x) => setV(Math.round(x)),
    })
    return () => controls.stop()
  }, [value])
  return <span className={className}>{v}</span>
}

function SummaryModal({
  open,
  number,
  stats,
  onAct,
  onBack,
}: {
  open: boolean
  number: string
  stats: InvStats
  onAct: () => void
  onBack: () => void
}) {
  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.18 }}
          className="fixed inset-0 z-[70] flex items-center justify-center bg-[#303466]/45 backdrop-blur-sm p-4"
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 12 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.97, y: 8 }}
            transition={{ type: 'spring', stiffness: 300, damping: 26 }}
            className="w-full max-w-[480px] rounded-modal bg-surface shadow-modal p-6"
          >
            <div className="text-center">
              <span className="mx-auto flex h-12 w-12 items-center justify-center rounded-full bg-success-bg text-success">
                <ClipboardCheck size={22} />
              </span>
              <h3 className="mt-3 text-[17px] leading-6 font-semibold text-ink-900">
                Сверка {number} завершена
              </h3>
              <p className="mt-1 text-[13px] leading-[18px] text-ink-500">
                Итоги инвентаризации
              </p>
            </div>

            <div className="mt-5 grid grid-cols-3 gap-3">
              <div className="rounded-xl bg-success-bg/60 p-3 text-center">
                <CountUp value={stats.matched} className="block text-2xl font-bold text-success" />
                <span className="text-caption text-success">Совпало</span>
              </div>
              <div className="rounded-xl bg-brand-50 p-3 text-center">
                <CountUp value={stats.surplus} className="block text-2xl font-bold text-brand-600" />
                <span className="text-caption text-brand-600">Излишки</span>
              </div>
              <div className="rounded-xl bg-danger-bg/60 p-3 text-center">
                <CountUp value={stats.shortage} className="block text-2xl font-bold text-danger" />
                <span className="text-caption text-danger">Недостача</span>
              </div>
            </div>

            <div className="mt-6 flex flex-col-reverse sm:flex-row justify-end gap-2">
              <button
                type="button"
                onClick={onBack}
                className="h-10 rounded-xl border border-brand-100 bg-surface px-5 text-sm font-semibold text-ink-900 transition-colors hover:bg-brand-50"
              >
                Вернуться
              </button>
              <button
                type="button"
                onClick={onAct}
                className="inline-flex h-10 items-center justify-center gap-2 rounded-xl bg-accent px-5 text-sm font-semibold text-white transition-all hover:bg-accent-hover active:scale-[0.97]"
              >
                <FileDown size={16} />
                Сформировать акт
              </button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}
