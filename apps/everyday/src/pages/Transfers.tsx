import { useEffect, useMemo, useRef, useState } from 'react'
import { Link } from 'react-router'
import {
  ArrowLeftRight,
  Camera,
  Check,
  CheckCircle2,
  Info,
  Loader2,
  Minus,
  Plus,
  Warehouse,
  X,
  XCircle,
} from 'lucide-react'
import { AnimatePresence, animate, motion } from 'framer-motion'
import { format } from 'date-fns'
import { ru } from 'date-fns/locale'
import { trpc } from '@/providers/trpc'
import type { inferRouterOutputs } from '@trpc/server'
import type { AppRouter } from '../../api/router'
import { cn } from '@/lib/utils'

// ─── Типы tRPC ───────────────────────────────────────────────────────────────

type RouterOutputs = inferRouterOutputs<AppRouter>
type Transfer = RouterOutputs['transfers']['outgoing'][number]
type ItemOption = RouterOutputs['items']['list']['rows'][number]
type UserOption = NonNullable<Transfer['toUser']>
type StorageOption = NonNullable<Transfer['toStorage']>

// ─── Утилиты ─────────────────────────────────────────────────────────────────

const fmtDateTime = (d: Date | string) => format(new Date(d), 'dd.MM.yyyy HH:mm', { locale: ru })

function titlePhoto(item: Transfer['item'] | ItemOption): string | null {
  const photos = 'photos' in item ? item.photos : []
  return photos.find((p) => p.isTitle)?.url ?? photos[0]?.url ?? null
}

function initials(name: string): string {
  return name
    .split(' ')
    .map((w) => w[0])
    .slice(0, 2)
    .join('')
    .toUpperCase()
}

/** Count-up число (design: count-up 500ms) */
function CountUp({ value, className }: { value: number; className?: string }) {
  const [display, setDisplay] = useState(value)
  const prevRef = useRef(value)
  useEffect(() => {
    const from = prevRef.current
    prevRef.current = value
    if (from === value) return
    const controls = animate(from, value, {
      duration: 0.5,
      onUpdate: (v) => setDisplay(Math.round(v)),
    })
    return () => controls.stop()
  }, [value])
  return <span className={className}>{display}</span>
}

// ─── Аватар ──────────────────────────────────────────────────────────────────

function Avatar({ name, url, size = 32 }: { name: string; url?: string | null; size?: number }) {
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
      style={{ width: size, height: size, fontSize: size * 0.36 }}
      className="rounded-full bg-brand-50 text-brand-600 font-semibold inline-flex items-center justify-center shrink-0 border border-brand-100"
    >
      {initials(name)}
    </span>
  )
}

// ─── Маршрут «ФИО или склад» (фирменный элемент с бегущей точкой) ────────────

function RouteViz({
  fromName,
  fromAvatar,
  toName,
  toAvatar,
  toIsStorage,
}: {
  fromName: string
  fromAvatar?: string | null
  toName: string
  toAvatar?: string | null
  toIsStorage?: boolean
}) {
  const trackRef = useRef<HTMLDivElement>(null)
  const [trackW, setTrackW] = useState(0)
  useEffect(() => {
    const el = trackRef.current
    if (!el) return
    const update = () => setTrackW(el.clientWidth)
    update()
    const ro = new ResizeObserver(update)
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  return (
    <div className="flex items-center gap-2 min-w-0">
      <div className="flex items-center gap-2 min-w-0">
        <Avatar name={fromName} url={fromAvatar} size={32} />
        <span className="text-[13px] text-ink-900 font-medium truncate max-w-[110px]">{fromName}</span>
      </div>
      <div ref={trackRef} className="relative flex-1 min-w-[48px] h-5">
        <div className="absolute inset-x-0 top-1/2 border-t border-dashed border-brand-100" />
        {/* Стрелка */}
        <svg
          className="absolute right-0 top-1/2 -translate-y-1/2 text-brand-100"
          width="10"
          height="10"
          viewBox="0 0 10 10"
          fill="currentColor"
        >
          <path d="M0 0L10 5L0 10V0Z" />
        </svg>
        {/* Бегущая точка (намёк на mesh-передачу) */}
        {trackW > 8 && (
          <motion.span
            className="absolute top-1/2 -mt-[3px] w-[6px] h-[6px] rounded-full bg-teal"
            animate={{ x: [0, trackW - 6], opacity: [0, 1, 1, 0] }}
            transition={{ duration: 2, repeat: Infinity, ease: 'linear', times: [0, 0.1, 0.9, 1] }}
          />
        )}
      </div>
      <div className="flex items-center gap-2 min-w-0">
        {toIsStorage ? (
          <span className="w-8 h-8 rounded-full bg-brand-50 text-brand-600 inline-flex items-center justify-center shrink-0 border border-brand-100">
            <Warehouse size={15} />
          </span>
        ) : (
          <Avatar name={toName} url={toAvatar} size={32} />
        )}
        <span className="text-[13px] text-ink-900 font-medium truncate max-w-[110px]">{toName}</span>
      </div>
    </div>
  )
}

// ─── Баннер «Внимание!» ───────────────────────────────────────────────────────

function InfoBanner({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div
      className={cn(
        'flex items-start gap-2.5 rounded-xl bg-info-bg border-l-[3px] border-teal px-3.5 py-3 text-sm text-ink-900',
        className
      )}
    >
      <Info size={16} className="text-teal-dark shrink-0 mt-0.5" />
      <p className="leading-snug">{children}</p>
    </div>
  )
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

// ─── Модальная обёртка ───────────────────────────────────────────────────────

function Modal({
  title,
  onClose,
  children,
  wide,
}: {
  title: string
  onClose: () => void
  children: React.ReactNode
  wide?: boolean
}) {
  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.18 }}
      className="fixed inset-0 z-[80] flex items-end sm:items-center justify-center p-3 bg-[rgba(48,52,102,.45)] backdrop-blur-sm"
      onClick={onClose}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.96, y: 12 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.97, y: 8 }}
        transition={{ type: 'spring', stiffness: 300, damping: 26 }}
        className={cn(
          'w-full bg-surface rounded-modal shadow-modal p-5 sm:p-6 max-h-[90dvh] overflow-y-auto',
          wide ? 'max-w-[560px]' : 'max-w-[440px]'
        )}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between gap-3 mb-4">
          <h3 className="text-[17px] leading-6 font-semibold text-ink-900">{title}</h3>
          <button
            onClick={onClose}
            className="w-8 h-8 rounded-lg inline-flex items-center justify-center text-ink-500 hover:bg-brand-50 transition-colors"
            aria-label="Закрыть"
          >
            <X size={16} />
          </button>
        </div>
        {children}
      </motion.div>
    </motion.div>
  )
}

// ─── Кастомный чекбокс (design.md §6) ────────────────────────────────────────

function Checkbox({
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
    <label
      className={cn(
        'inline-flex items-start gap-2.5 select-none',
        disabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'
      )}
    >
      <span
        role="checkbox"
        aria-checked={checked}
        tabIndex={disabled ? -1 : 0}
        onClick={() => !disabled && onChange(!checked)}
        onKeyDown={(e) => {
          if (!disabled && (e.key === ' ' || e.key === 'Enter')) {
            e.preventDefault()
            onChange(!checked)
          }
        }}
        className={cn(
          'w-5 h-5 rounded-md border-[1.5px] inline-flex items-center justify-center shrink-0 transition-colors mt-px',
          checked ? 'bg-brand-600 border-brand-600' : 'bg-surface border-brand-100 hover:border-brand-600'
        )}
      >
        <AnimatePresence>
          {checked && (
            <motion.span
              initial={{ scale: 0.4, opacity: 0 }}
              animate={{ scale: 1, opacity: 1 }}
              exit={{ scale: 0.4, opacity: 0 }}
              transition={{ duration: 0.15 }}
            >
              <Check size={13} strokeWidth={3} className="text-white" />
            </motion.span>
          )}
        </AnimatePresence>
      </span>
      {label && <span className="text-sm text-ink-900 leading-snug">{label}</span>}
    </label>
  )
}

// ─── Фото инструмента ────────────────────────────────────────────────────────

function ItemPhoto({ item, size = 72 }: { item: Transfer['item']; size?: number }) {
  const url = titlePhoto(item)
  if (url) {
    return (
      <img
        src={url}
        alt={item.title}
        style={{ width: size, height: size }}
        className="rounded-[10px] object-cover shrink-0 border border-brand-100/60 bg-brand-50"
      />
    )
  }
  return (
    <span
      style={{ width: size, height: size }}
      className="rounded-[10px] bg-brand-50 text-brand-600 inline-flex items-center justify-center shrink-0"
    >
      <ArrowLeftRight size={size * 0.32} />
    </span>
  )
}

// ─── Карточка исходящей передачи (таб «Отдать») ─────────────────────────────

function OutgoingCard({
  transfer,
  index,
  onCancel,
  cancelling,
}: {
  transfer: Transfer
  index: number
  onCancel: (t: Transfer) => void
  cancelling: boolean
}) {
  const item = transfer.item
  const toIsStorage = !!transfer.toStorage
  const toName = transfer.toStorage?.name ?? transfer.toUser?.fullName ?? '—'
  const toAvatar = transfer.toStorage ? null : (transfer.toUser?.avatarUrl ?? null)
  const partial = transfer.quantity != null && item.quantitative

  return (
    <motion.article
      layout="position"
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{
        opacity: 0,
        height: 0,
        marginBottom: 0,
        overflow: 'hidden',
        transition: { duration: 0.3, delay: 0 },
      }}
      transition={{ duration: 0.3, delay: Math.min(index, 11) * 0.06 }}
      className="bg-surface rounded-card border border-brand-100/60 shadow-card p-5 space-y-3.5"
    >
      <div className="flex items-start gap-4">
        <ItemPhoto item={item} />
        <div className="flex-1 min-w-0">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <Link
                to={`/tool/${item.id}`}
                className="text-[15px] leading-[22px] font-semibold text-ink-900 hover:text-brand-600 transition-colors line-clamp-2"
              >
                {item.title}
              </Link>
              <div className="font-mono-num text-ink-500 mt-0.5">{item.internalId}</div>
            </div>
            <span className="inline-flex items-center rounded-full px-2.5 py-0.5 text-caption bg-warning-bg text-warning shrink-0">
              В процессе
            </span>
          </div>
        </div>
      </div>

      {/* Маршрут */}
      <RouteViz
        fromName={transfer.fromUser?.fullName ?? 'Я'}
        fromAvatar={transfer.fromUser?.avatarUrl}
        toName={toName}
        toAvatar={toAvatar}
        toIsStorage={toIsStorage}
      />

      {partial && (
        <>
          <div className="text-sm text-ink-900">
            Количество:{' '}
            <span className="font-mono-num">
              {transfer.quantity} из {item.quantity ?? '—'} {item.unit ?? 'шт'}
            </span>
          </div>
          <InfoBanner>
            Частичная передача — партия разделена, остаток{' '}
            <span className="font-mono-num">
              {Math.max(0, (item.quantity ?? 0) - (transfer.quantity ?? 0))} {item.unit ?? 'шт'}
            </span>
          </InfoBanner>
        </>
      )}

      {transfer.comment && <p className="text-[13px] text-ink-500">«{transfer.comment}»</p>}

      <div className="flex flex-wrap items-center justify-between gap-3 pt-1">
        <div className="text-[13px] text-ink-500">
          Создана <span className="font-mono-num">{fmtDateTime(transfer.createdAt)}</span>
          {transfer.code && (
            <>
              {' · '}ID <span className="font-mono-num">{transfer.code}</span>
            </>
          )}
        </div>
        <button
          onClick={() => onCancel(transfer)}
          disabled={cancelling}
          className="inline-flex items-center gap-1.5 text-sm font-semibold text-danger hover:bg-danger-bg rounded-lg px-3 py-1.5 transition-colors disabled:opacity-50"
        >
          <X size={15} />
          Отменить передачу
        </button>
      </div>
    </motion.article>
  )
}

// ─── Карточка входящей передачи (таб «Принять») ─────────────────────────────

interface AcceptPayload {
  comment?: string
  photoUrl?: string
}

function IncomingCard({
  transfer,
  index,
  selected,
  onToggleSelected,
  onAccept,
  onReject,
  busy,
  acceptedFlash,
}: {
  transfer: Transfer
  index: number
  selected: boolean
  onToggleSelected: (id: number, v: boolean) => void
  onAccept: (id: number, payload: AcceptPayload) => void
  onReject: (t: Transfer) => void
  busy: boolean
  acceptedFlash: boolean
}) {
  const item = transfer.item
  const [comment, setComment] = useState('')
  const [photos, setPhotos] = useState<string[]>([])
  const [inspected, setInspected] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const fileRef = useRef<HTMLInputElement>(null)

  const canAccept = photos.length > 0 || inspected

  const addPhotos = (files: FileList | null) => {
    if (!files) return
    const urls = Array.from(files)
      .filter((f) => f.type.startsWith('image/'))
      .map((f) => URL.createObjectURL(f))
    if (urls.length) {
      setPhotos((prev) => [...prev, ...urls])
      setError(null)
    }
  }

  const handleAccept = () => {
    if (!canAccept) {
      setError('Подтвердите осмотр или приложите фото дефектов')
      return
    }
    onAccept(transfer.id, {
      comment: comment.trim() || undefined,
      photoUrl: photos[0],
    })
  }

  return (
    <motion.article
      layout="position"
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{
        opacity: 0,
        height: 0,
        marginBottom: 0,
        overflow: 'hidden',
        transition: { duration: 0.3, delay: 0 },
      }}
      transition={{ duration: 0.3, delay: Math.min(index, 11) * 0.06 }}
      className={cn(
        'bg-surface rounded-card border shadow-card p-5 space-y-3.5 transition-colors',
        acceptedFlash ? 'border-success bg-success-bg/40' : 'border-brand-100/60'
      )}
    >
      <div className="flex items-start gap-4">
        <div className="pt-1">
          <Checkbox checked={selected} onChange={(v) => onToggleSelected(transfer.id, v)} disabled={acceptedFlash} />
        </div>
        <ItemPhoto item={item} />
        <div className="flex-1 min-w-0">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <Link
                to={`/tool/${item.id}`}
                className="text-[15px] leading-[22px] font-semibold text-ink-900 hover:text-brand-600 transition-colors line-clamp-2"
              >
                {item.title}
              </Link>
              <div className="text-[13px] text-ink-500 mt-0.5">
                <span className="font-mono-num">{item.internalId}</span>
                {' · '}от {transfer.fromUser?.fullName ?? '—'}
                {' · '}Создана <span className="font-mono-num">{fmtDateTime(transfer.createdAt)}</span>
                {transfer.code && (
                  <>
                    {' · '}<span className="font-mono-num">{transfer.code}</span>
                  </>
                )}
              </div>
            </div>
            {acceptedFlash ? (
              <span className="inline-flex items-center gap-1 rounded-full px-2.5 py-0.5 text-caption bg-success-bg text-success shrink-0">
                <Check size={12} strokeWidth={3} />
                Принято
              </span>
            ) : (
              <span className="inline-flex items-center rounded-full px-2.5 py-0.5 text-caption bg-warning-bg text-warning shrink-0">
                В процессе
              </span>
            )}
          </div>
        </div>
      </div>

      {transfer.quantity != null && item.quantitative && (
        <div className="text-sm text-ink-900">
          Количество:{' '}
          <span className="font-mono-num">
            {transfer.quantity} {item.unit ?? 'шт'}
          </span>{' '}
          <span className="text-ink-500 text-[13px]">(частичная передача партии)</span>
        </div>
      )}

      {!acceptedFlash && (
        <>
          <InfoBanner>
            Осмотрите инструмент при получении. Зафиксируйте дефекты фото — это защитит вас.
          </InfoBanner>

          <div className="space-y-1.5">
            <label className="text-[13px] font-semibold text-ink-500">Комментарий приёмки</label>
            <textarea
              value={comment}
              onChange={(e) => setComment(e.target.value)}
              rows={2}
              placeholder="Состояние, замечания, комплектность…"
              className="w-full rounded-lg border border-brand-100 bg-surface px-4 py-2.5 text-sm text-ink-900 placeholder:text-ink-300 focus:border-brand-600 focus:ring-[0_0_0_3px_#5E629B22] focus:outline-none resize-y min-h-[56px]"
            />
          </div>

          <div className="space-y-2">
            <div className="text-[13px] font-semibold text-ink-500">Фото дефектов</div>
            <div className="flex flex-wrap items-center gap-2">
              {photos.map((url, i) => (
                <span key={url} className="relative">
                  <img
                    src={url}
                    alt={`Дефект ${i + 1}`}
                    className="w-[72px] h-[72px] rounded-[10px] object-cover border border-brand-100"
                  />
                  <button
                    onClick={() => setPhotos((prev) => prev.filter((_, j) => j !== i))}
                    className="absolute -top-1.5 -right-1.5 w-5 h-5 rounded-full bg-ink-900 text-white inline-flex items-center justify-center shadow-card"
                    aria-label="Удалить фото"
                  >
                    <X size={11} />
                  </button>
                </span>
              ))}
              <button
                onClick={() => fileRef.current?.click()}
                className="w-[72px] h-[72px] rounded-[10px] border border-dashed border-brand-100 text-brand-600 hover:border-brand-600 hover:bg-brand-50 transition-colors inline-flex flex-col items-center justify-center gap-1"
              >
                <Camera size={18} />
                <span className="text-[11px] font-semibold">Фото</span>
              </button>
              <input
                ref={fileRef}
                type="file"
                accept="image/*"
                multiple
                className="hidden"
                onChange={(e) => {
                  addPhotos(e.target.files)
                  e.target.value = ''
                }}
              />
            </div>
          </div>

          {photos.length === 0 && (
            <Checkbox
              checked={inspected}
              onChange={(v) => {
                setInspected(v)
                if (v) setError(null)
              }}
              label="Осмотрено, претензий нет"
            />
          )}

          {error && <p className="text-xs font-semibold text-danger">{error}</p>}

          <div className="flex flex-wrap items-center justify-end gap-2.5 pt-1">
            <button
              onClick={() => onReject(transfer)}
              disabled={busy}
              className="h-10 px-5 rounded-lg border-[1.5px] border-danger text-danger text-sm font-semibold hover:bg-danger-bg transition-colors disabled:opacity-50"
            >
              Отказ
            </button>
            <motion.button
              whileHover={{ y: -1 }}
              whileTap={{ scale: 0.97 }}
              onClick={handleAccept}
              disabled={busy}
              className="h-10 px-5 rounded-lg bg-accent text-white text-sm font-semibold inline-flex items-center gap-2 hover:bg-accent-hover transition-colors disabled:opacity-50"
            >
              {busy ? <Loader2 size={16} className="animate-spin" /> : <Check size={16} />}
              Принять
            </motion.button>
          </div>
        </>
      )}
    </motion.article>
  )
}

// ─── Модалка «Передать инструмент» ───────────────────────────────────────────

function NewTransferModal({
  onClose,
  onCreated,
}: {
  onClose: () => void
  onCreated: (code: string | null, toName: string) => void
}) {
  const utils = trpc.useUtils()
  const itemsQ = trpc.items.list.useQuery({ limit: 100 })
  const usersQ = trpc.admin.users.list.useQuery({})
  const storagesQ = trpc.admin.storages.list.useQuery({})
  const meQ = trpc.meta.currentUser.useQuery()

  const [itemId, setItemId] = useState<number | null>(null)
  const [route, setRoute] = useState<'user' | 'storage'>('user')
  const [toUserId, setToUserId] = useState<number | null>(null)
  const [toStorageId, setToStorageId] = useState<number | null>(null)
  const [quantity, setQuantity] = useState(1)
  const [comment, setComment] = useState('')
  const [needPhoto, setNeedPhoto] = useState(true)
  const [error, setError] = useState<string | null>(null)

  const prepare = trpc.transfers.prepare.useMutation({
    onSuccess: (t) => {
      utils.transfers.outgoing.invalidate()
      utils.transfers.incoming.invalidate()
      utils.meta.transferCounts.invalidate()
      utils.history.movements.invalidate()
      const toName =
        t?.toStorage?.name ?? t?.toUser?.fullName ?? ''
      onCreated(t?.code ?? null, toName)
    },
    onError: (e) => setError(e.message),
  })

  const items = useMemo(
    () =>
      (itemsQ.data?.rows ?? []).filter((i) => i.status?.slug !== 'written-off'),
    [itemsQ.data]
  )
  const users: UserOption[] = useMemo(
    () => (usersQ.data ?? []).filter((u) => u.id !== meQ.data?.id && u.status !== 'disabled'),
    [usersQ.data, meQ.data]
  )
  const storages: StorageOption[] = useMemo(() => storagesQ.data ?? [], [storagesQ.data])

  const item = items.find((i) => i.id === itemId) ?? null
  const maxQty = item?.quantitative ? (item.quantity ?? 1) : 1

  useEffect(() => {
    if (quantity <= maxQty) return
    const frame = requestAnimationFrame(() => setQuantity(maxQty))
    return () => cancelAnimationFrame(frame)
  }, [maxQty, quantity])

  const storageResponsible = storages.find((s) => s.id === toStorageId)?.responsibleUserId ?? null
  const effectiveToUser = route === 'user' ? toUserId : storageResponsible

  const submit = () => {
    if (!itemId) return setError('Выберите инструмент')
    if (!effectiveToUser)
      return setError(route === 'user' ? 'Выберите сотрудника' : 'У склада нет ответственного — выберите другой')
    setError(null)
    prepare.mutate({
      itemId,
      toUserId: effectiveToUser,
      toStorageId: route === 'storage' ? toStorageId : undefined,
      quantity: item?.quantitative ? quantity : undefined,
      comment: comment.trim() || undefined,
      noConfirmation: !needPhoto,
    })
  }

  return (
    <Modal title="Передать инструмент" onClose={onClose} wide>
      <div className="space-y-4">
        {/* Инструмент */}
        <div className="space-y-1.5">
          <label className="text-[13px] font-semibold text-ink-500">
            Инструмент <span className="text-accent">*</span>
          </label>
          {item && (
            <div className="flex items-center gap-3 rounded-lg border border-brand-100 p-2.5 mb-2">
              <ItemPhoto item={item} size={48} />
              <div className="min-w-0">
                <div className="text-sm font-semibold text-ink-900 truncate">{item.title}</div>
                <div className="font-mono-num text-ink-500">{item.internalId}</div>
              </div>
            </div>
          )}
          <select
            value={itemId ?? ''}
            onChange={(e) => setItemId(e.target.value ? Number(e.target.value) : null)}
            className="w-full h-11 rounded-lg border border-brand-100 bg-surface px-4 text-sm text-ink-900 focus:border-brand-600 focus:outline-none"
          >
            <option value="">Выберите инструмент…</option>
            {items.map((i) => (
              <option key={i.id} value={i.id}>
                {i.internalId} — {i.title}
              </option>
            ))}
          </select>
        </div>

        {/* Кому: сегмент Сотруднику / На склад */}
        <div className="space-y-1.5">
          <label className="text-[13px] font-semibold text-ink-500">
            Кому <span className="text-accent">*</span>
          </label>
          <div className="flex rounded-lg bg-brand-50 p-1 gap-1">
            {(
              [
                { key: 'user', label: 'Сотруднику' },
                { key: 'storage', label: 'На склад' },
              ] as const
            ).map((s) => (
              <button
                key={s.key}
                onClick={() => setRoute(s.key)}
                className={cn(
                  'relative flex-1 h-9 rounded-lg text-sm font-semibold transition-colors',
                  route === s.key ? 'text-ink-900' : 'text-ink-500 hover:text-ink-900'
                )}
              >
                {route === s.key && (
                  <motion.span
                    layoutId="transfer-route-segment"
                    className="absolute inset-0 bg-surface rounded-lg shadow-card"
                    transition={{ duration: 0.2 }}
                  />
                )}
                <span className="relative z-10">{s.label}</span>
              </button>
            ))}
          </div>

          {route === 'user' ? (
            <div className="max-h-44 overflow-y-auto rounded-lg border border-brand-100 divide-y divide-brand-100/50">
              {users.length === 0 && (
                <div className="px-4 py-3 text-sm text-ink-500">
                  {usersQ.isLoading ? 'Загрузка…' : 'Нет доступных сотрудников'}
                </div>
              )}
              {users.map((u) => (
                <button
                  key={u.id}
                  onClick={() => setToUserId(u.id)}
                  className={cn(
                    'w-full flex items-center gap-3 px-3 py-2 text-left transition-colors',
                    toUserId === u.id ? 'bg-brand-50' : 'hover:bg-brand-50/60'
                  )}
                >
                  <Avatar name={u.fullName} url={u.avatarUrl} size={28} />
                  <span className="flex-1 min-w-0">
                    <span className="block text-sm font-medium text-ink-900 truncate">{u.fullName}</span>
                    {u.position && <span className="block text-xs text-ink-500">{u.position}</span>}
                  </span>
                  {toUserId === u.id && <Check size={16} className="text-brand-600 shrink-0" />}
                </button>
              ))}
            </div>
          ) : (
            <div className="max-h-44 overflow-y-auto rounded-lg border border-brand-100 divide-y divide-brand-100/50">
              {storages.length === 0 && (
                <div className="px-4 py-3 text-sm text-ink-500">
                  {storagesQ.isLoading ? 'Загрузка…' : 'Нет складов'}
                </div>
              )}
              {storages.map((s) => (
                <button
                  key={s.id}
                  onClick={() => setToStorageId(s.id)}
                  className={cn(
                    'w-full flex items-center gap-3 px-3 py-2 text-left transition-colors',
                    toStorageId === s.id ? 'bg-brand-50' : 'hover:bg-brand-50/60'
                  )}
                >
                  <span className="w-7 h-7 rounded-full bg-brand-50 text-brand-600 inline-flex items-center justify-center shrink-0">
                    <Warehouse size={14} />
                  </span>
                  <span className="flex-1 min-w-0">
                    <span className="block text-sm font-medium text-ink-900 truncate">{s.name}</span>
                    {s.address && <span className="block text-xs text-ink-500">{s.address}</span>}
                  </span>
                  {toStorageId === s.id && <Check size={16} className="text-brand-600 shrink-0" />}
                </button>
              ))}
            </div>
          )}
        </div>

        {/* Количество (для материалов) */}
        {item?.quantitative && (
          <div className="space-y-1.5">
            <label className="text-[13px] font-semibold text-ink-500">Количество</label>
            <div className="flex items-center gap-3">
              <div className="inline-flex items-center rounded-lg border border-brand-100 overflow-hidden">
                <button
                  onClick={() => setQuantity((q) => Math.max(1, q - 1))}
                  className="w-10 h-11 inline-flex items-center justify-center text-ink-500 hover:bg-brand-50"
                  aria-label="Меньше"
                >
                  <Minus size={15} />
                </button>
                <span className="w-14 text-center font-mono-num text-ink-900">{quantity}</span>
                <button
                  onClick={() => setQuantity((q) => Math.min(maxQty, q + 1))}
                  className="w-10 h-11 inline-flex items-center justify-center text-ink-500 hover:bg-brand-50"
                  aria-label="Больше"
                >
                  <Plus size={15} />
                </button>
              </div>
              <span className="text-[13px] text-ink-500">
                из <span className="font-mono-num">{maxQty}</span> {item.unit ?? 'шт'}
              </span>
            </div>
            {quantity < maxQty && (
              <p className="text-xs text-teal-dark">Передача части разделит партию</p>
            )}
          </div>
        )}

        {/* Комментарий */}
        <div className="space-y-1.5">
          <label className="text-[13px] font-semibold text-ink-500">Комментарий</label>
          <textarea
            value={comment}
            onChange={(e) => setComment(e.target.value)}
            rows={2}
            placeholder="Куда и зачем передаётся…"
            className="w-full rounded-lg border border-brand-100 bg-surface px-4 py-2.5 text-sm text-ink-900 placeholder:text-ink-300 focus:border-brand-600 focus:outline-none resize-y min-h-[56px]"
          />
        </div>

        <Checkbox
          checked={needPhoto}
          onChange={setNeedPhoto}
          label="Требуется фотофиксация при приёмке"
        />

        {error && <p className="text-xs font-semibold text-danger">{error}</p>}

        <div className="flex items-center justify-end gap-2.5 pt-1">
          <button
            onClick={onClose}
            className="h-10 px-5 rounded-lg border border-brand-100 bg-surface text-sm font-semibold text-ink-900 hover:bg-brand-50 transition-colors"
          >
            Отмена
          </button>
          <motion.button
            whileHover={{ y: -1 }}
            whileTap={{ scale: 0.97 }}
            onClick={submit}
            disabled={prepare.isPending}
            className="h-10 px-5 rounded-lg bg-accent text-white text-sm font-semibold inline-flex items-center gap-2 hover:bg-accent-hover transition-colors disabled:opacity-50"
          >
            {prepare.isPending && <Loader2 size={16} className="animate-spin" />}
            Передать
          </motion.button>
        </div>
      </div>
    </Modal>
  )
}

// ─── Empty state ─────────────────────────────────────────────────────────────

function EmptyState({
  title,
  subtitle,
  cta,
}: {
  title: string
  subtitle: string
  cta?: { label: string; to: string }
}) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 16 }}
      animate={{ opacity: 1, y: 0 }}
      className="bg-surface rounded-card border border-brand-100/60 shadow-card px-6 py-12 flex flex-col items-center text-center gap-3"
    >
      <img src="/empty-transfers.svg" alt="" className="w-[240px] max-w-full" />
      <h3 className="text-[17px] leading-6 font-semibold text-ink-900">{title}</h3>
      <p className="text-[13px] text-ink-500 max-w-[340px]">{subtitle}</p>
      {cta && (
        <Link
          to={cta.to}
          className="mt-1 h-10 px-5 rounded-lg border border-brand-100 bg-surface text-sm font-semibold text-ink-900 hover:bg-brand-50 transition-colors inline-flex items-center"
        >
          {cta.label}
        </Link>
      )}
    </motion.div>
  )
}

// ─── Модалка отказа (приёмка) ────────────────────────────────────────────────

function RejectModal({
  transfer,
  onClose,
  onSubmit,
  busy,
}: {
  transfer: Transfer
  onClose: () => void
  onSubmit: (comment: string) => void
  busy: boolean
}) {
  const [reason, setReason] = useState('')
  const [photos, setPhotos] = useState<string[]>([])
  const [error, setError] = useState<string | null>(null)
  const fileRef = useRef<HTMLInputElement>(null)

  const submit = () => {
    if (!reason.trim()) return setError('Укажите причину отказа')
    if (photos.length === 0) return setError('Приложите фото дефекта — это обязательно при отказе')
    onSubmit(reason.trim())
  }

  return (
    <Modal title="Указать причину отказа" onClose={onClose}>
      <div className="space-y-4">
        <div className="flex items-center gap-3 rounded-lg border border-brand-100 p-2.5">
          <ItemPhoto item={transfer.item} size={48} />
          <div className="min-w-0">
            <div className="text-sm font-semibold text-ink-900 truncate">{transfer.item.title}</div>
            <div className="font-mono-num text-ink-500">{transfer.item.internalId}</div>
          </div>
        </div>
        <div className="space-y-1.5">
          <label className="text-[13px] font-semibold text-ink-500">
            Причина <span className="text-accent">*</span>
          </label>
          <textarea
            value={reason}
            onChange={(e) => {
              setReason(e.target.value)
              setError(null)
            }}
            rows={3}
            placeholder="Что не так с инструментом…"
            className={cn(
              'w-full rounded-lg border bg-surface px-4 py-2.5 text-sm text-ink-900 placeholder:text-ink-300 focus:outline-none resize-y min-h-[72px]',
              error && !reason.trim() ? 'border-danger' : 'border-brand-100 focus:border-brand-600'
            )}
          />
        </div>
        <div className="space-y-2">
          <div className="text-[13px] font-semibold text-ink-500">
            Фото дефекта <span className="text-accent">*</span>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            {photos.map((url, i) => (
              <span key={url} className="relative">
                <img
                  src={url}
                  alt={`Дефект ${i + 1}`}
                  className="w-[72px] h-[72px] rounded-[10px] object-cover border border-brand-100"
                />
                <button
                  onClick={() => setPhotos((prev) => prev.filter((_, j) => j !== i))}
                  className="absolute -top-1.5 -right-1.5 w-5 h-5 rounded-full bg-ink-900 text-white inline-flex items-center justify-center shadow-card"
                  aria-label="Удалить фото"
                >
                  <X size={11} />
                </button>
              </span>
            ))}
            <button
              onClick={() => fileRef.current?.click()}
              className="w-[72px] h-[72px] rounded-[10px] border border-dashed border-brand-100 text-brand-600 hover:border-brand-600 hover:bg-brand-50 transition-colors inline-flex flex-col items-center justify-center gap-1"
            >
              <Camera size={18} />
              <span className="text-[11px] font-semibold">Фото</span>
            </button>
            <input
              ref={fileRef}
              type="file"
              accept="image/*"
              multiple
              className="hidden"
              onChange={(e) => {
                const urls = Array.from(e.target.files ?? [])
                  .filter((f) => f.type.startsWith('image/'))
                  .map((f) => URL.createObjectURL(f))
                if (urls.length) {
                  setPhotos((prev) => [...prev, ...urls])
                  setError(null)
                }
                e.target.value = ''
              }}
            />
          </div>
        </div>
        {error && <p className="text-xs font-semibold text-danger">{error}</p>}
        <div className="flex items-center justify-end gap-2.5">
          <button
            onClick={onClose}
            className="h-10 px-5 rounded-lg border border-brand-100 bg-surface text-sm font-semibold text-ink-900 hover:bg-brand-50 transition-colors"
          >
            Назад
          </button>
          <motion.button
            whileTap={{ scale: 0.97 }}
            onClick={submit}
            disabled={busy}
            className="h-10 px-5 rounded-lg bg-danger text-white text-sm font-semibold inline-flex items-center gap-2 hover:opacity-90 transition-opacity disabled:opacity-50"
          >
            {busy && <Loader2 size={16} className="animate-spin" />}
            Отказать в приёмке
          </motion.button>
        </div>
      </div>
    </Modal>
  )
}

// ─── Страница ────────────────────────────────────────────────────────────────

export default function Transfers() {
  const utils = trpc.useUtils()
  const [tab, setTab] = useState<'out' | 'in'>('out')

  const outgoingQ = trpc.transfers.outgoing.useQuery()
  const incomingQ = trpc.transfers.incoming.useQuery()
  const countsQ = trpc.meta.transferCounts.useQuery()

  const outgoing = useMemo(() => outgoingQ.data ?? [], [outgoingQ.data])
  const incoming = useMemo(() => incomingQ.data ?? [], [incomingQ.data])
  const outCount = countsQ.data?.outgoing ?? outgoing.length
  const inCount = countsQ.data?.incoming ?? incoming.length

  const [toast, setToast] = useState<{ text: string; tone: 'ok' | 'err' } | null>(null)
  useEffect(() => {
    if (!toast) return
    const t = setTimeout(() => setToast(null), 4000)
    return () => clearTimeout(t)
  }, [toast])

  const [cancelTarget, setCancelTarget] = useState<Transfer | null>(null)
  const [rejectTarget, setRejectTarget] = useState<Transfer | null>(null)
  const [showNew, setShowNew] = useState(false)
  const [acceptedFlash, setAcceptedFlash] = useState<Set<number>>(new Set())
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set())
  const knownRef = useRef<Set<number>>(new Set())
  const [bulkBusy, setBulkBusy] = useState(false)
  const [cancelBusy, setCancelBusy] = useState(false)

  // Новые входящие выбираем по умолчанию, явный снятый выбор сохраняем
  useEffect(() => {
    setSelectedIds((prev) => {
      const next = new Set<number>()
      for (const t of incoming) {
        next.add(t.id)
        if (knownRef.current.has(t.id) && !prev.has(t.id)) next.delete(t.id)
      }
      return next
    })
    knownRef.current = new Set(incoming.map((t) => t.id))
  }, [incoming])

  const invalidateAll = () => {
    utils.transfers.incoming.invalidate()
    utils.transfers.outgoing.invalidate()
    utils.meta.transferCounts.invalidate()
    utils.items.list.invalidate()
    utils.history.movements.invalidate()
    utils.history.all.invalidate()
  }

  const flashAccepted = (id: number) => {
    setAcceptedFlash((prev) => new Set(prev).add(id))
    setTimeout(() => {
      setAcceptedFlash((prev) => {
        const next = new Set(prev)
        next.delete(id)
        return next
      })
    }, 1200)
  }

  const acceptMut = trpc.transfers.accept.useMutation({
    onSuccess: (_d, vars) => {
      flashAccepted(vars.id)
      setToast({ text: 'Принято ✓ · записано в журнал операций', tone: 'ok' })
      invalidateAll()
    },
    onError: (e) => setToast({ text: e.message, tone: 'err' }),
  })

  const rejectMut = trpc.transfers.reject.useMutation({
    onSuccess: () => {
      setToast({ text: 'Отказ отправлен отправителю', tone: 'ok' })
      invalidateAll()
    },
    onError: (e) => setToast({ text: e.message, tone: 'err' }),
  })

  const acceptAllMut = trpc.transfers.acceptAll.useMutation({
    onSuccess: (d) => {
      setToast({ text: `Принято ${d.acceptedCount} ед. · записано в журнал операций`, tone: 'ok' })
      invalidateAll()
    },
    onError: (e) => setToast({ text: e.message, tone: 'err' }),
  })

  const handleAccept = (id: number, payload: AcceptPayload) => {
    acceptMut.mutate({ id, comment: payload.comment, photoUrl: payload.photoUrl })
  }

  const handleConfirmAll = async () => {
    const ids = incoming.filter((t) => selectedIds.has(t.id)).map((t) => t.id)
    if (ids.length === 0) return
    setBulkBusy(true)
    try {
      if (ids.length === incoming.length) {
        await acceptAllMut.mutateAsync()
      } else {
        for (const id of ids) {
          flashAccepted(id)
          await acceptMut.mutateAsync({ id })
        }
        setToast({ text: `Принято ${ids.length} ед. · записано в журнал операций`, tone: 'ok' })
      }
    } catch {
      /* onError мутации покажет тост */
    } finally {
      setBulkBusy(false)
    }
  }

  const handleCancelOutgoing = async () => {
    if (!cancelTarget) return
    setCancelBusy(true)
    try {
      await rejectMut.mutateAsync({ id: cancelTarget.id, comment: 'Отменено отправителем' })
      setToast({ text: 'Передача отменена', tone: 'ok' })
      setCancelTarget(null)
    } catch {
      /* тост из onError */
    } finally {
      setCancelBusy(false)
    }
  }

  const loading = tab === 'out' ? outgoingQ.isLoading : incomingQ.isLoading
  const loadError = tab === 'out' ? outgoingQ.error : incomingQ.error

  const tabs = [
    { key: 'out' as const, label: 'Отдать', count: outCount },
    { key: 'in' as const, label: 'Принять', count: inCount },
  ]

  return (
    <div className="space-y-5">
      {/* ─── Секция 1. Заголовок и сводка ─── */}
      <motion.div
        initial={{ opacity: 0, y: 16 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.28 }}
        className="flex flex-wrap items-center justify-between gap-3"
      >
        <h1 className="text-2xl lg:text-[28px] leading-9 font-bold tracking-[-0.01em] text-ink-900">
          Приём-передача
        </h1>
        <div className="flex items-center gap-2.5 flex-wrap">
          <button
            onClick={() => setShowNew(true)}
            className="h-10 px-4 rounded-lg border border-brand-100 bg-surface text-sm font-semibold text-ink-900 hover:bg-brand-50 transition-colors inline-flex items-center gap-2"
          >
            <ArrowLeftRight size={16} className="text-brand-600" />
            Новая передача
          </button>
          <div className="rounded-lg bg-warning-bg px-5 py-3 text-sm text-ink-900">
            На передачу{' '}
            <CountUp value={outCount} className="font-mono-num text-warning" /> шт. · Ожидает приёма{' '}
            <CountUp value={inCount} className="font-mono-num text-warning" /> шт.
          </div>
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
                layoutId="transfers-tab-underline"
                className="absolute left-0 right-0 -bottom-px h-0.5 bg-accent rounded-full"
                transition={{ duration: 0.2 }}
              />
            )}
          </button>
        ))}
      </div>

      {/* ─── Секции 3–4. Списки передач ─── */}
      {loading ? (
        <div className="space-y-4">
          {[0, 1].map((i) => (
            <div
              key={i}
              className="bg-surface rounded-card border border-brand-100/60 shadow-card p-5 h-44 animate-pulse"
            >
              <div className="flex gap-4">
                <div className="w-[72px] h-[72px] rounded-[10px] bg-brand-50" />
                <div className="flex-1 space-y-2.5">
                  <div className="h-4 w-2/3 rounded bg-brand-50" />
                  <div className="h-3 w-1/3 rounded bg-brand-50" />
                  <div className="h-3 w-1/2 rounded bg-brand-50" />
                </div>
              </div>
            </div>
          ))}
        </div>
      ) : loadError ? (
        <div className="bg-surface rounded-card border border-danger/40 shadow-card p-6 text-sm text-danger flex items-center justify-between gap-3">
          <span>Не удалось загрузить передачи: {loadError.message}</span>
          <button
            onClick={() => (tab === 'out' ? outgoingQ.refetch() : incomingQ.refetch())}
            className="h-9 px-4 rounded-lg border border-brand-100 text-ink-900 font-semibold hover:bg-brand-50"
          >
            Повторить
          </button>
        </div>
      ) : tab === 'out' ? (
        outgoing.length === 0 ? (
          <EmptyState
            title="Нет исходящих передач"
            subtitle="Передайте инструмент из каталога или карточки"
            cta={{ label: 'Перейти в каталог', to: '/' }}
          />
        ) : (
          <div className="space-y-4">
            <AnimatePresence>
              {outgoing.map((t, i) => (
                <OutgoingCard
                  key={t.id}
                  transfer={t}
                  index={i}
                  onCancel={setCancelTarget}
                  cancelling={cancelBusy}
                />
              ))}
            </AnimatePresence>
          </div>
        )
      ) : incoming.length === 0 ? (
        <EmptyState
          title="Вам ничего не передают"
          subtitle="Когда коллега передаст вам инструмент, он появится здесь"
        />
      ) : (
        <div className="space-y-4">
          <AnimatePresence>
            {incoming.map((t, i) => (
              <IncomingCard
                key={t.id}
                transfer={t}
                index={i}
                selected={selectedIds.has(t.id)}
                onToggleSelected={(id, v) =>
                  setSelectedIds((prev) => {
                    const next = new Set(prev)
                    if (v) next.add(id)
                    else next.delete(id)
                    return next
                  })
                }
                onAccept={handleAccept}
                onReject={setRejectTarget}
                busy={acceptMut.isPending || bulkBusy}
                acceptedFlash={acceptedFlash.has(t.id)}
              />
            ))}
          </AnimatePresence>

          {/* Футер таба: массовая приёмка */}
          <div className="sticky bottom-3 z-30">
            <motion.div
              initial={{ opacity: 0, y: 16 }}
              animate={{ opacity: 1, y: 0 }}
              className="mx-auto max-w-xl bg-surface rounded-card border border-brand-100/60 shadow-hover px-5 py-3.5 flex items-center justify-between gap-3"
            >
              <span className="text-sm text-ink-500">
                Выбрано{' '}
                <span className="font-mono-num text-ink-900">{selectedIds.size}</span> из{' '}
                <span className="font-mono-num text-ink-900">{incoming.length}</span>
              </span>
              <motion.button
                whileHover={{ y: -1 }}
                whileTap={{ scale: 0.97 }}
                onClick={handleConfirmAll}
                disabled={bulkBusy || selectedIds.size === 0}
                className="h-10 px-5 rounded-lg bg-accent text-white text-sm font-semibold inline-flex items-center gap-2 hover:bg-accent-hover transition-colors disabled:opacity-50"
              >
                {bulkBusy ? <Loader2 size={16} className="animate-spin" /> : <Check size={16} />}
                Подтвердить все ({selectedIds.size})
              </motion.button>
            </motion.div>
          </div>
        </div>
      )}

      {/* ─── Модалки ─── */}
      <AnimatePresence>
        {cancelTarget && (
          <Modal
            key="cancel"
            title="Отменить передачу?"
            onClose={() => !cancelBusy && setCancelTarget(null)}
          >
            <div className="space-y-4">
              <div className="flex items-center gap-3 rounded-lg border border-brand-100 p-2.5">
                <ItemPhoto item={cancelTarget.item} size={48} />
                <div className="min-w-0">
                  <div className="text-sm font-semibold text-ink-900 truncate">
                    {cancelTarget.item.title}
                  </div>
                  <div className="font-mono-num text-ink-500">
                    {cancelTarget.item.internalId}
                    {cancelTarget.code ? ` · ${cancelTarget.code}` : ''}
                  </div>
                </div>
              </div>
              <p className="text-sm text-ink-500">
                Получатель увидит отказ, а инструмент останется у вас. Действие будет записано в
                журнал операций.
              </p>
              <div className="flex items-center justify-end gap-2.5">
                <button
                  onClick={() => setCancelTarget(null)}
                  disabled={cancelBusy}
                  className="h-10 px-5 rounded-lg border border-brand-100 bg-surface text-sm font-semibold text-ink-900 hover:bg-brand-50 transition-colors"
                >
                  Назад
                </button>
                <motion.button
                  whileTap={{ scale: 0.97 }}
                  onClick={handleCancelOutgoing}
                  disabled={cancelBusy}
                  className="h-10 px-5 rounded-lg bg-danger text-white text-sm font-semibold inline-flex items-center gap-2 hover:opacity-90 transition-opacity disabled:opacity-50"
                >
                  {cancelBusy && <Loader2 size={16} className="animate-spin" />}
                  Отменить передачу
                </motion.button>
              </div>
            </div>
          </Modal>
        )}
        {rejectTarget && (
          <RejectModal
            key="reject"
            transfer={rejectTarget}
            onClose={() => setRejectTarget(null)}
            busy={rejectMut.isPending}
            onSubmit={(comment) =>
              rejectMut.mutate(
                { id: rejectTarget.id, comment },
                { onSuccess: () => setRejectTarget(null) }
              )
            }
          />
        )}
        {showNew && (
          <NewTransferModal
            key="new"
            onClose={() => setShowNew(false)}
            onCreated={(code, toName) => {
              setShowNew(false)
              setToast({
                text: `Передача ${code ?? ''} создана · ожидает приёмки ${toName}`.trim(),
                tone: 'ok',
              })
              setTab('out')
            }}
          />
        )}
      </AnimatePresence>

      {/* ─── Тост ─── */}
      <AnimatePresence>{toast && <Toast text={toast.text} tone={toast.tone} />}</AnimatePresence>
    </div>
  )
}
