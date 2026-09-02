import { useEffect, useMemo, useRef, useState } from 'react'
import type { ChangeEvent, ReactNode } from 'react'
import { Link, useNavigate, useParams } from 'react-router'
import { AnimatePresence, motion } from 'framer-motion'
import useEmblaCarousel from 'embla-carousel-react'
import {
  ArrowLeft,
  ArrowLeftRight,
  Building2,
  CheckCircle2,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ClipboardCheck,
  Copy,
  FileImage,
  FileText,
  History as HistoryIcon,
  Loader2,
  MessageSquare,
  Minus,
  MinusCircle,
  Pencil,
  Plus,
  PlusCircle,
  Printer,
  QrCode,
  Send,
  Star,
  Trash2,
  Upload,
  Warehouse,
  Wrench,
  X,
  ZoomIn,
  PackageCheck,
  Undo2,
  AlertTriangle,
  CalendarClock,
} from 'lucide-react'
import { QRCodeCanvas, QRCodeSVG } from 'qrcode.react'
import { format } from 'date-fns'
import { ru } from 'date-fns/locale'
import { cn } from '@/lib/utils'
import { trpc } from '@/providers/trpc'
import { askStatusReason, itemCirculates, statusNeedsReason } from '@/lib/status-reason'
import { parseDueInput, toDateTimeLocal } from '@/lib/due-date'
import { preparePhoto } from '@/lib/photo'
import { useStore } from '@/lib/store'
import QrScanner from '@/components/QrScanner'
import { BROWSER_FILE_LIMIT_BYTES, BROWSER_FILE_LIMIT_LABEL } from '@/lib/content-limits'

// ─── Утилиты ─────────────────────────────────────────────────────────────────

const fmtDate = (d: Date | string) => format(new Date(d), 'dd.MM.yyyy', { locale: ru })
const fmtDateTime = (d: Date | string) => format(new Date(d), 'dd.MM.yyyy · HH:mm', { locale: ru })
const fmtMoney = (v: number | null | undefined) =>
  v == null ? '—' : `${new Intl.NumberFormat('ru-RU').format(v)} ₽`

const HISTORY_META: Record<string, { label: string; icon: typeof Star; color: string }> = {
  create: { label: 'Создание карточки', icon: Star, color: '#5E629B' },
  update: { label: 'Редактирование', icon: Pencil, color: '#5E629B' },
  move: { label: 'Перемещение', icon: ArrowLeftRight, color: '#2E8E86' },
  transfer_send: { label: 'Передача', icon: ArrowLeftRight, color: '#2E8E86' },
  transfer_receive: { label: 'Приём', icon: ArrowLeftRight, color: '#2E9E5B' },
  replenish: { label: 'Пополнение', icon: PlusCircle, color: '#2E9E5B' },
  write_off: { label: 'Списание', icon: MinusCircle, color: '#D64545' },
  inventory: { label: 'Инвентаризация', icon: ClipboardCheck, color: '#5E629B' },
}

// eslint-disable-next-line @typescript-eslint/no-unused-vars
function useItemData(id: number) {
  return trpc.items.byId.useQuery({ id }, { enabled: Number.isFinite(id) && id > 0 })
}
type ItemFull = NonNullable<ReturnType<typeof useItemData>['data']>

// ─── Тост (паттерн из Catalog) ────────────────────────────────────────────────

function Toast({ text }: { text: string | null }) {
  return (
    <AnimatePresence>
      {text && (
        <motion.div
          initial={{ opacity: 0, y: 24 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 12 }}
          transition={{ duration: 0.24 }}
          className="fixed bottom-24 lg:bottom-8 left-1/2 -translate-x-1/2 z-[70] inline-flex items-center gap-2 rounded-full bg-ink-900 text-white text-sm font-semibold px-5 py-3 shadow-modal"
        >
          <CheckCircle2 size={16} className="text-teal" />
          {text}
        </motion.div>
      )}
    </AnimatePresence>
  )
}

function useToast() {
  const [toast, setToast] = useState<string | null>(null)
  useEffect(() => {
    if (!toast) return
    const t = setTimeout(() => setToast(null), 4000)
    return () => clearTimeout(t)
  }, [toast])
  return { toast, showToast: setToast }
}

// ─── Модалка (design.md §6: spring scale 0.96→1) ─────────────────────────────

function Modal({
  open,
  onClose,
  title,
  children,
  wide,
}: {
  open: boolean
  onClose: () => void
  title: string
  children: ReactNode
  wide?: boolean
}) {
  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.18 }}
          className="fixed inset-0 z-[60] flex items-end sm:items-center justify-center p-0 sm:p-6 bg-[rgba(48,52,102,.45)] backdrop-blur-[4px]"
          onClick={onClose}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 12 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 12 }}
            transition={{ type: 'spring', stiffness: 300, damping: 26 }}
            onClick={(e) => e.stopPropagation()}
            className={cn(
              'w-full bg-surface rounded-t-modal sm:rounded-modal shadow-modal p-5 sm:p-6 max-h-[90dvh] overflow-y-auto',
              wide ? 'max-w-[720px]' : 'max-w-[560px]'
            )}
          >
            <div className="flex items-center justify-between gap-3 mb-4">
              <h3 className="text-[17px] leading-6 font-semibold text-ink-900">{title}</h3>
              <button
                onClick={onClose}
                aria-label="Закрыть"
                className="w-9 h-9 shrink-0 flex items-center justify-center rounded-xl text-ink-500 hover:bg-brand-50 transition-colors"
              >
                <X size={18} strokeWidth={1.75} />
              </button>
            </div>
            {children}
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}

// ─── Кнопки дизайн-системы ────────────────────────────────────────────────────

function PrimaryButton({
  children,
  className,
  ...rest
}: React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      {...rest}
      className={cn(
        'inline-flex items-center justify-center gap-2 h-10 px-5 rounded-xl bg-accent text-white text-sm font-semibold transition-all duration-150 hover:bg-accent-hover hover:-translate-y-px active:scale-[0.97] disabled:opacity-50 disabled:pointer-events-none',
        className
      )}
    >
      {children}
    </button>
  )
}

function SecondaryButton({
  children,
  className,
  ...rest
}: React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      {...rest}
      className={cn(
        'inline-flex items-center justify-center gap-2 h-10 px-4 rounded-xl bg-surface border border-brand-100 text-ink-900 text-sm font-semibold transition-all duration-150 hover:bg-brand-50 active:scale-[0.97] disabled:opacity-50 disabled:pointer-events-none',
        className
      )}
    >
      {children}
    </button>
  )
}

function IconButton({
  children,
  className,
  danger,
  ...rest
}: React.ButtonHTMLAttributes<HTMLButtonElement> & { danger?: boolean }) {
  return (
    <button
      {...rest}
      className={cn(
        'w-10 h-10 inline-flex items-center justify-center rounded-xl border transition-all duration-150 active:scale-[0.97] disabled:opacity-50 disabled:pointer-events-none',
        danger
          ? 'bg-surface border-danger/40 text-danger hover:bg-danger-bg'
          : 'bg-surface border-brand-100 text-ink-900 hover:bg-brand-50',
        className
      )}
    >
      {children}
    </button>
  )
}

// ─── Статус-бейдж по данным API (slug/color/bg) ──────────────────────────────

function ApiStatusBadge({ status, className }: { status: { name: string; color: string; bg: string } | null | undefined; className?: string }) {
  if (!status) return <span className={cn('text-sm text-ink-300', className)}>—</span>
  return (
    <span
      className={cn('inline-flex items-center rounded-full px-2.5 py-0.5 text-caption', className)}
      style={{ background: status.bg, color: status.color }}
    >
      {status.name}
    </span>
  )
}

// ─── Секция 2. Фотогалерея (embla + lightbox с зумом) ────────────────────────

function PhotoGallery({
  photos,
  status,
  hasQr,
  onAddLocal,
}: {
  photos: string[]
  status: { name: string; color: string; bg: string } | null | undefined
  hasQr: boolean
  onAddLocal: (url: string) => void
}) {
  const [emblaRef, emblaApi] = useEmblaCarousel({ loop: false, align: 'start' })
  const [selected, setSelected] = useState(0)
  const [lightbox, setLightbox] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    if (!emblaApi) return
    const onSelect = () => setSelected(emblaApi.selectedScrollSnap())
    emblaApi.on('select', onSelect)
    onSelect()
    return () => {
      emblaApi.off('select', onSelect)
    }
  }, [emblaApi])

  // Листать к последнему кадру при добавлении фото
  const prevCount = useRef(photos.length)
  useEffect(() => {
    if (photos.length > prevCount.current && emblaApi) {
      emblaApi.scrollTo(photos.length - 1)
    }
    prevCount.current = photos.length
  }, [photos.length, emblaApi])

  const pickFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const f = e.target.files?.[0]
    if (f) onAddLocal(URL.createObjectURL(f))
    e.target.value = ''
  }

  return (
    <motion.section
      initial={{ opacity: 0, scale: 0.98 }}
      animate={{ opacity: 1, scale: 1 }}
      transition={{ duration: 0.3, ease: 'easeOut' }}
      className="bg-surface rounded-card border border-brand-100/60 shadow-card p-4"
    >
      <div className="relative">
        {photos.length === 0 ? (
          <div className="aspect-[4/3] rounded-[16px] bg-brand-50 flex flex-col items-center justify-center gap-2 text-ink-300">
            <Wrench size={40} strokeWidth={1.25} />
            <span className="text-sm font-semibold text-ink-500">Фото пока нет</span>
          </div>
        ) : (
          <div ref={emblaRef} className="overflow-hidden rounded-[16px]">
            <div className="flex">
              {photos.map((src, i) => (
                <div key={`${src}-${i}`} className="min-w-0 flex-[0_0_100%]">
                  <button
                    onClick={() => setLightbox(true)}
                    className="block w-full aspect-[4/3] bg-brand-50 overflow-hidden group/zoom"
                    aria-label="Открыть фото на весь экран"
                  >
                    <img
                      src={src}
                      alt={`Фото ${i + 1}`}
                      className="w-full h-full object-cover transition-transform duration-300 group-hover/zoom:scale-[1.02]"
                      draggable={false}
                    />
                  </button>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Бейджи поверх фото */}
        <div className="absolute right-3 top-3 flex gap-1.5 pointer-events-none">
          {hasQr && (
            <span className="inline-flex items-center gap-1 rounded-full bg-teal/20 backdrop-blur-sm px-2 py-0.5 text-[11px] font-semibold text-teal-dark">
              <QrCode size={12} strokeWidth={1.75} />
              QR
            </span>
          )}
          <ApiStatusBadge status={status} />
        </div>
        {photos.length > 0 && (
          <div className="absolute left-3 bottom-3 pointer-events-none">
            <span className="inline-flex items-center gap-1 rounded-full bg-ink-900/60 backdrop-blur-sm px-2 py-1 text-[11px] font-semibold text-white">
              <ZoomIn size={12} strokeWidth={1.75} />
              Нажмите для зума
            </span>
          </div>
        )}

        {/* Стрелки */}
        {photos.length > 1 && (
          <>
            <button
              onClick={() => emblaApi?.scrollPrev()}
              aria-label="Предыдущее фото"
              className="absolute left-3 top-1/2 -translate-y-1/2 w-10 h-10 rounded-full bg-surface shadow-card flex items-center justify-center text-ink-900 hover:bg-brand-50 transition-colors"
            >
              <ChevronLeft size={18} strokeWidth={1.75} />
            </button>
            <button
              onClick={() => emblaApi?.scrollNext()}
              aria-label="Следующее фото"
              className="absolute right-3 top-1/2 -translate-y-1/2 w-10 h-10 rounded-full bg-surface shadow-card flex items-center justify-center text-ink-900 hover:bg-brand-50 transition-colors"
            >
              <ChevronRight size={18} strokeWidth={1.75} />
            </button>
          </>
        )}
      </div>

      {/* Миниатюры + добавить */}
      <div className="flex items-center gap-2 mt-3 overflow-x-auto pb-1">
        {photos.map((src, i) => (
          <button
            key={`${src}-thumb-${i}`}
            onClick={() => emblaApi?.scrollTo(i)}
            aria-label={`Фото ${i + 1}`}
            className={cn(
              'w-16 h-12 shrink-0 rounded-lg overflow-hidden border-2 transition-all duration-150',
              selected === i ? 'border-accent' : 'border-transparent opacity-70 hover:opacity-100'
            )}
          >
            <img src={src} alt="" className="w-full h-full object-cover" draggable={false} />
          </button>
        ))}
        <button
          onClick={() => fileRef.current?.click()}
          className="h-12 shrink-0 inline-flex items-center gap-1.5 px-3 rounded-lg border border-brand-100 bg-surface text-xs font-semibold text-ink-500 hover:bg-brand-50 transition-colors"
        >
          <Plus size={14} strokeWidth={1.75} />
          Добавить фото
        </button>
        <input ref={fileRef} type="file" accept="image/*" className="hidden" onChange={pickFile} />
      </div>

      {/* Точки-индикаторы (мобайл) */}
      {photos.length > 1 && (
        <div className="flex lg:hidden justify-center gap-1.5 mt-2">
          {photos.map((_, i) => (
            <span
              key={i}
              className={cn(
                'w-1.5 h-1.5 rounded-full transition-colors',
                selected === i ? 'bg-accent' : 'bg-brand-100'
              )}
            />
          ))}
        </div>
      )}

      <Lightbox open={lightbox} src={photos[selected]} onClose={() => setLightbox(false)} />
    </motion.section>
  )
}

// Lightbox: overlay 90%, зум до 2× колесом/кликом, перетаскивание, закрытие крестиком/Esc
function Lightbox({ open, src, onClose }: { open: boolean; src: string | undefined; onClose: () => void }) {
  const [zoomed, setZoomed] = useState(false)

  const close = () => {
    setZoomed(false)
    onClose()
  }

  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, onClose])

  return (
    <AnimatePresence>
      {open && src && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.2 }}
          className="fixed inset-0 z-[80] bg-ink-900/90 flex items-center justify-center p-4 sm:p-10"
          onClick={close}
        >
          <button
            onClick={close}
            aria-label="Закрыть"
            className="absolute right-4 top-4 z-10 w-10 h-10 rounded-full bg-white/10 hover:bg-white/20 text-white flex items-center justify-center transition-colors"
          >
            <X size={20} strokeWidth={1.75} />
          </button>
          <motion.div
            initial={{ scale: 0.9, opacity: 0 }}
            animate={{ scale: 1, opacity: 1 }}
            exit={{ scale: 0.9, opacity: 0 }}
            transition={{ type: 'spring', stiffness: 300, damping: 26 }}
            className="max-w-full max-h-full overflow-hidden rounded-card"
            onClick={(e) => e.stopPropagation()}
            onWheel={(e) => setZoomed(e.deltaY < 0)}
          >
            <motion.img
              src={src}
              alt="Фото инструмента"
              draggable={false}
              drag={zoomed}
              dragConstraints={{ left: -300, right: 300, top: -300, bottom: 300 }}
              dragElastic={0.1}
              animate={{ scale: zoomed ? 2 : 1 }}
              transition={{ duration: 0.24, ease: [0.22, 1, 0.36, 1] }}
              onClick={() => setZoomed((z) => !z)}
              className={cn(
                'max-w-[92vw] max-h-[84dvh] object-contain select-none',
                zoomed ? 'cursor-grab active:cursor-grabbing' : 'cursor-zoom-in'
              )}
            />
          </motion.div>
          <div className="absolute bottom-5 left-1/2 -translate-x-1/2 text-white/70 text-xs">
            Клик — зум 2× · перетаскивание — сдвиг · Esc — закрыть
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  )
}

// ─── Секция 3. Модалки действий ──────────────────────────────────────────────

function TransferModal({
  open,
  onClose,
  item,
  onDone,
}: {
  open: boolean
  onClose: () => void
  item: ItemFull
  onDone: (msg: string) => void
}) {
  const utils = trpc.useUtils()
  const { data: users } = trpc.admin.users.list.useQuery({})
  const { data: storages } = trpc.admin.storages.list.useQuery({})
  const [toUserId, setToUserId] = useState<number | ''>('')
  const [toStorageId, setToStorageId] = useState<number | ''>('')
  const [comment, setComment] = useState('')
  const [quantity, setQuantity] = useState<number>(1)

  const prepare = trpc.transfers.prepare.useMutation({
    onSuccess: (t) => {
      utils.transfers.outgoing.invalidate()
      utils.transfers.incoming.invalidate()
      utils.meta.transferCounts.invalidate()
      utils.items.byId.invalidate({ id: item.id })
      onDone(`Передача ${t?.code ?? ''} создана`)
      onClose()
      setToUserId('')
      setToStorageId('')
      setComment('')
      setQuantity(1)
    },
  })

  const candidates = (users ?? []).filter((u) => u.id !== item.responsibleUserId && u.status === 'active')

  return (
    <Modal open={open} onClose={onClose} title={`Передать: ${item.title}`}>
      <div className="space-y-4">
        <div>
          <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">Кому *</label>
          <select
            value={toUserId}
            onChange={(e) => setToUserId(e.target.value ? Number(e.target.value) : '')}
            className="w-full h-11 rounded-xl border border-brand-100 bg-surface px-3 text-[15px] text-ink-900 focus:border-brand-600 focus:ring-[3px] focus:ring-brand-600/15"
          >
            <option value="">Выберите сотрудника</option>
            {candidates.map((u) => (
              <option key={u.id} value={u.id}>
                {u.fullName}
                {u.position ? ` · ${u.position}` : ''}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">На склад (если возврат)</label>
          <select
            value={toStorageId}
            onChange={(e) => setToStorageId(e.target.value ? Number(e.target.value) : '')}
            className="w-full h-11 rounded-xl border border-brand-100 bg-surface px-3 text-[15px] text-ink-900 focus:border-brand-600 focus:ring-[3px] focus:ring-brand-600/15"
          >
            <option value="">Не менять</option>
            {(storages ?? []).map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
              </option>
            ))}
          </select>
        </div>
        {item.quantitative && (
          <div>
            <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">
              Количество ({item.unit ?? 'шт'}), доступно {item.quantity ?? 0}
            </label>
            <input
              type="number"
              min={0.001}
              max={item.quantity ?? undefined}
              step="any"
              value={quantity}
              onChange={(e) => setQuantity(Number(e.target.value))}
              className="w-full h-11 rounded-xl border border-brand-100 bg-surface px-3 font-mono-num text-ink-900 focus:border-brand-600 focus:ring-[3px] focus:ring-brand-600/15"
            />
          </div>
        )}
        <div>
          <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">Комментарий</label>
          <textarea
            value={comment}
            onChange={(e) => setComment(e.target.value)}
            rows={2}
            placeholder="Например: на объект до конца недели"
            className="w-full rounded-xl border border-brand-100 bg-surface px-3 py-2.5 text-[15px] text-ink-900 placeholder:text-ink-300 focus:border-brand-600 focus:ring-[3px] focus:ring-brand-600/15 resize-none"
          />
        </div>
        <div className="rounded-xl bg-info-bg border-l-[3px] border-teal px-3 py-2.5 text-sm text-ink-900">
          Получателю придёт уведомление. Инструмент перейдёт к нему после подтверждения приёма.
        </div>
        <div className="flex justify-end gap-2">
          <SecondaryButton onClick={onClose}>Отмена</SecondaryButton>
          <PrimaryButton
            disabled={!toUserId || prepare.isPending}
            onClick={() =>
              prepare.mutate({
                itemId: item.id,
                toUserId: Number(toUserId),
                toStorageId: toStorageId === '' ? undefined : Number(toStorageId),
                quantity: item.quantitative ? quantity : undefined,
                comment: comment || undefined,
              })
            }
          >
            {prepare.isPending ? <Loader2 size={16} className="animate-spin" /> : <ArrowLeftRight size={16} strokeWidth={1.75} />}
            Создать передачу
          </PrimaryButton>
        </div>
      </div>
    </Modal>
  )
}

function QrModal({ open, onClose, item }: { open: boolean; onClose: () => void; item: ItemFull }) {
  const utils = trpc.useUtils()
  const labelQ = trpc.items.qrLabel.useQuery({ itemId: item.id }, { enabled: open, retry: false })
  const value = labelQ.data?.label
  const [binding, setBinding] = useState(false)
  const [existingQr, setExistingQr] = useState('')
  const [bindingMessage, setBindingMessage] = useState('')
  const bindQr = trpc.items.update.useMutation({
    onSuccess: async () => {
      await utils.items.byId.invalidate({ id: item.id })
      await utils.items.list.invalidate()
      setBinding(false)
      setExistingQr('')
      setBindingMessage('Существующий QR привязан подписанной операцией')
    },
    onError: (error) => setBindingMessage(error.message),
  })

  const downloadPng = () => {
    const canvas = document.getElementById('item-qr-canvas') as HTMLCanvasElement | null
    if (!canvas) return
    const a = document.createElement('a')
    a.href = canvas.toDataURL('image/png')
    a.download = `qr-${item.internalId}.png`
    a.click()
  }

  const print = () => {
    const canvas = document.getElementById('item-qr-canvas') as HTMLCanvasElement | null
    if (!canvas) return
    const w = window.open('', '_blank', 'width=400,height=520')
    if (!w) return
    w.document.write(
      `<html><head><title>QR ${item.internalId}</title></head><body style="font-family:sans-serif;text-align:center;padding:24px">` +
        `<img src="${canvas.toDataURL('image/png')}" style="width:220px"/>` +
        `<div style="font-family:monospace;font-weight:600;margin-top:12px">${item.internalId}</div>` +
        `<div style="font-size:14px;margin-top:4px">${item.title}</div>` +
        `</body></html>`
    )
    w.document.close()
    w.focus()
    w.print()
  }

  return (
    <Modal open={open} onClose={onClose} title="QR-код инструмента">
      <div className="flex flex-col items-center gap-4">
        {value ? <>
          <div className="bg-white rounded-card border border-brand-100 p-5 shadow-card">
            <QRCodeSVG value={value} size={200} level="M" />
          </div>
          {/* Скрытый canvas для экспорта PNG */}
          <div className="hidden">
            <QRCodeCanvas id="item-qr-canvas" value={value} size={512} level="M" />
          </div>
        </> : <div className="flex h-60 items-center justify-center text-ink-500">
          {labelQ.isError ? 'Не удалось подписать QR' : <Loader2 className="animate-spin" size={24} />}
        </div>}
        <div className="font-mono-num text-ink-900">{item.internalId}</div>
        <p className="text-[13px] text-ink-500 text-center">
          Метка подписана Ed25519 и привязана к организации и карточке. Копию
          настоящей наклейки всё равно можно сделать — сверяйте название и номер на корпусе.
        </p>
        {item.qrCode && item.qrCode !== item.internalId && (
          <p className="w-full break-all rounded-xl bg-teal/10 p-3 text-xs text-teal-dark">
            Уже привязана внешняя метка: <span className="font-mono-num">{item.qrCode}</span>
          </p>
        )}
        <SecondaryButton
          onClick={() => {
            setBinding((current) => !current)
            setExistingQr('')
            setBindingMessage('')
          }}
          className="w-full"
        >
          <QrCode size={16} />
          {binding ? 'Отмена привязки' : 'Привязать уже наклеенный QR'}
        </SecondaryButton>
        {binding && (
          <div className="w-full space-y-3" data-testid="existing-qr-rebind">
            <QrScanner onCode={(code) => setExistingQr(code.trim())} subjectLabel="существующую метку ТМЦ" />
            {existingQr && (
              <p className="break-all rounded-xl bg-brand-50 p-3 font-mono-num text-xs">{existingQr}</p>
            )}
            <PrimaryButton
              className="w-full"
              disabled={!existingQr || bindQr.isPending}
              onClick={() => bindQr.mutate({ id: item.id, qrCode: existingQr })}
            >
              {bindQr.isPending ? <Loader2 size={16} className="animate-spin" /> : <CheckCircle2 size={16} />}
              Подтвердить привязку
            </PrimaryButton>
          </div>
        )}
        {bindingMessage && <p className="w-full text-center text-sm text-ink-700">{bindingMessage}</p>}
        <div className="flex gap-2 w-full">
          <SecondaryButton onClick={downloadPng} className="flex-1">
            Скачать PNG
          </SecondaryButton>
          <PrimaryButton onClick={print} className="flex-1">
            <Printer size={16} strokeWidth={1.75} />
            Печать
          </PrimaryButton>
        </div>
      </div>
    </Modal>
  )
}

function DeleteModal({
  open,
  onClose,
  item,
  onDeleted,
}: {
  open: boolean
  onClose: () => void
  item: ItemFull
  onDeleted: () => void
}) {
  const utils = trpc.useUtils()
  const remove = trpc.items.remove.useMutation({
    onSuccess: () => {
      utils.items.list.invalidate()
      onDeleted()
    },
  })
  return (
    <Modal open={open} onClose={onClose} title="Удалить карточку?">
      <p className="text-[15px] leading-[22px] text-ink-900">
        <span className="font-mono-num">{item.internalId}</span> {item.title} будет удалён из каталога.
        История операций сохранится в журнале.
      </p>
      <div className="flex justify-end gap-2 mt-6">
        <SecondaryButton onClick={onClose}>Отмена</SecondaryButton>
        <button
          onClick={() => remove.mutate({ id: item.id })}
          disabled={remove.isPending}
          className="inline-flex items-center gap-2 h-10 px-5 rounded-xl bg-danger text-white text-sm font-semibold transition-all duration-150 hover:brightness-95 active:scale-[0.97] disabled:opacity-50"
        >
          {remove.isPending ? <Loader2 size={16} className="animate-spin" /> : <Trash2 size={16} strokeWidth={1.75} />}
          Удалить
        </button>
      </div>
    </Modal>
  )
}

// Пополнение / списание для количественного учёта
function QuantityModal({
  open,
  onClose,
  item,
  mode,
  onDone,
}: {
  open: boolean
  onClose: () => void
  item: ItemFull
  mode: 'replenish' | 'writeOff'
  onDone: (msg: string) => void
}) {
  const utils = trpc.useUtils()
  const { workspace } = useStore()
  const [qty, setQty] = useState(1)
  const [comment, setComment] = useState('')
  const [photo, setPhoto] = useState<string | null>(null)
  const [operationGuid, setOperationGuid] = useState(() => crypto.randomUUID())
  const [uploadError, setUploadError] = useState<string | null>(null)
  const isReplenish = mode === 'replenish'
  const photoRequired =
    !isReplenish &&
    workspace?.id === item.workspaceId && workspace.requireWriteoffPhoto === true

  const onSuccess = () => {
    utils.items.byId.invalidate({ id: item.id })
    utils.items.list.invalidate()
    utils.history.quantityOps.invalidate()
    onDone(isReplenish ? 'Остаток пополнен' : 'Количество списано')
    onClose()
    setQty(1)
    setComment('')
    setPhoto(null)
    setOperationGuid(crypto.randomUUID())
    setUploadError(null)
  }
  const replenish = trpc.history.replenish.useMutation({ onSuccess })
  const writeOff = trpc.history.writeOff.useMutation({ onSuccess })
  const ingestContent = trpc.content.ingest.useMutation()
  const pending = replenish.isPending || writeOff.isPending || ingestContent.isPending

  const submit = async () => {
    if (isReplenish) {
      if (!workspace?.guid || workspace.id !== item.workspaceId || !item.guid) {
        setUploadError('Нет глобального идентификатора организации или ТМЦ')
        return
      }
      replenish.mutate({
        itemId: item.id,
        workspaceGuid: workspace.guid,
        itemGuid: item.guid,
        operationGuid,
        quantity: qty,
        comment: comment || undefined,
      })
      return
    }
    setUploadError(null)
    try {
      if (!workspace?.guid || workspace.id !== item.workspaceId || !item.guid) {
        throw new Error('Не удалось определить GUID организации или карточки')
      }
      const uploaded = photo
        ? await ingestContent.mutateAsync({
            workspaceId: item.workspaceId,
            itemId: item.id,
            purpose: 'writeoff-photo',
            operationGuid,
            dataUrl: photo,
          })
        : null
      await writeOff.mutateAsync({
        itemId: item.id,
        workspaceGuid: workspace.guid,
        itemGuid: item.guid,
        operationGuid,
        quantity: qty,
        comment: comment.trim(),
        photoUrl: uploaded?.url,
      })
    } catch (error) {
      setUploadError(error instanceof Error ? error.message : 'Не удалось сохранить списание')
    }
  }

  return (
    <Modal open={open} onClose={onClose} title={isReplenish ? 'Пополнить остаток' : 'Списать количество'}>
      <div className="space-y-4">
        <div>
          <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">
            Количество ({item.unit ?? 'шт'}), сейчас {item.quantity ?? 0}
          </label>
          <div className="flex items-center gap-2">
            <IconButton onClick={() => setQty((q) => Math.max(1, q - 1))} aria-label="Меньше">
              <Minus size={16} strokeWidth={1.75} />
            </IconButton>
            <input
              type="number"
              min={1}
              step="any"
              value={qty}
              onChange={(e) => setQty(Math.max(0, Number(e.target.value)))}
              className="w-28 h-11 text-center rounded-xl border border-brand-100 bg-surface font-mono-num text-ink-900 focus:border-brand-600 focus:ring-[3px] focus:ring-brand-600/15"
            />
            <IconButton onClick={() => setQty((q) => q + 1)} aria-label="Больше">
              <Plus size={16} strokeWidth={1.75} />
            </IconButton>
          </div>
        </div>
        <div>
          <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">Комментарий</label>
          <textarea
            value={comment}
            onChange={(e) => setComment(e.target.value)}
            rows={2}
            placeholder={isReplenish ? 'Откуда поступление' : 'Причина списания'}
            className="w-full rounded-xl border border-brand-100 bg-surface px-3 py-2.5 text-[15px] text-ink-900 placeholder:text-ink-300 focus:border-brand-600 focus:ring-[3px] focus:ring-brand-600/15 resize-none"
          />
        </div>
        {!isReplenish && (
          <div>
            <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">
              Фото-подтверждение{photoRequired ? '' : ' (необязательно)'}
            </label>
            <input
              type="file"
              accept="image/*"
              capture="environment"
              className="block w-full text-[13px] text-ink-500"
              onChange={(e) => {
                const file = e.target.files?.[0]
                if (!file) return
                // Снимок с телефона весит мегабайты и уезжает в базу как
                // data-URL — уменьшаем перед отправкой.
                void preparePhoto(file).then((p) => setPhoto(p.url))
              }}
            />
            {photo && <img src={photo} alt="" className="mt-2 h-20 rounded-lg object-cover" />}
            {photoRequired && !photo && (
              <p className="mt-1 text-[13px] text-danger">
                В этой группе списание принимается только с фото.
              </p>
            )}
            {uploadError && <p className="mt-2 text-[13px] text-danger">{uploadError}</p>}
          </div>
        )}
        <div className="flex justify-end gap-2">
          <SecondaryButton onClick={onClose}>Отмена</SecondaryButton>
          <PrimaryButton
            disabled={
              qty <= 0 ||
              pending ||
              (!isReplenish && !comment.trim()) ||
              (photoRequired && !photo)
            }
            onClick={() => void submit()}
          >
            {pending && <Loader2 size={16} className="animate-spin" />}
            {isReplenish ? 'Пополнить' : 'Списать'}
          </PrimaryButton>
        </div>
      </div>
    </Modal>
  )
}

function TakeModal({
  open,
  onClose,
  item,
  onDone,
}: {
  open: boolean
  onClose: () => void
  item: ItemFull
  onDone: (msg: string) => void
}) {
  const utils = trpc.useUtils()
  const [comment, setComment] = useState('')
  const [noDue, setNoDue] = useState(false)
  const [photoUrl, setPhotoUrl] = useState<string | undefined>()
  const [qrLabel, setQrLabel] = useState('')
  const maxQty = item.quantitative
    ? Math.max(1, Number(item.quantity ?? 1))
    : Math.max(1, Number((item as { family?: { inStock?: number } }).family?.inStock ?? 1))
  const [qty, setQty] = useState(1)
  const defaultDue = () => {
    const d = new Date()
    d.setDate(d.getDate() + 1)
    d.setMinutes(0, 0, 0)
    return toDateTimeLocal(d)
  }
  const [dueLocal, setDueLocal] = useState(defaultDue)

  const take = trpc.transfers.take.useMutation({
    onSuccess: (res) => {
      utils.items.byId.invalidate({ id: item.id })
      utils.items.list.invalidate()
      utils.meta.transferCounts.invalidate()
      const pending = Boolean((res as { pending?: boolean } | null)?.pending)
      onDone(pending ? 'Заявка отправлена администратору' : 'Инструмент теперь у вас')
      onClose()
      setComment('')
      setQrLabel('')
    },
    onError: (e) => onDone(e.message),
  })

  return (
    <Modal open={open} onClose={onClose} title={`Взять: ${item.title}`}>
      <div className="space-y-4">
        <div>
          <p className="mb-2 text-[13px] font-semibold text-ink-500">
            Обязательно отсканируйте подписанную или ранее привязанную QR-бирку на самом оборудовании
          </p>
          <QrScanner onCode={(value) => setQrLabel(value.trim())} />
          {qrLabel && <p className="mt-2 text-xs font-semibold text-success">QR получен и будет проверен сервером</p>}
        </div>
        <label className="flex items-center gap-2 text-sm font-semibold text-ink-900">
          <input type="checkbox" checked={noDue} onChange={(e) => setNoDue(e.target.checked)} />
          Без срока возврата
        </label>
        {maxQty > 1 && (
          <div>
            <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">
              Количество {item.quantitative ? `(на складе ${item.quantity ?? 0} ${item.unit ?? 'шт'})` : `(свободно ${maxQty} шт)`}
            </label>
            <div className="flex items-center gap-2">
              <IconButton onClick={() => setQty((q) => Math.max(1, q - 1))} aria-label="Меньше">
                <Minus size={16} />
              </IconButton>
              <input
                type="number"
                min={1}
                max={maxQty}
                value={qty}
                onChange={(e) => setQty(Math.min(maxQty, Math.max(1, Number(e.target.value) || 1)))}
                className="w-24 h-11 text-center rounded-xl border border-brand-100 font-mono-num"
              />
              <IconButton onClick={() => setQty((q) => Math.min(maxQty, q + 1))} aria-label="Больше">
                <Plus size={16} />
              </IconButton>
            </div>
          </div>
        )}
        {!noDue && (
          <div>
            <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">Срок возврата</label>
            <input
              type="datetime-local"
              value={dueLocal}
              onChange={(e) => setDueLocal(e.target.value)}
              className="w-full h-11 rounded-xl border border-brand-100 bg-surface px-3 text-[15px] text-ink-900"
            />
          </div>
        )}
        <div>
          <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">Назначение / комментарий</label>
          <textarea
            value={comment}
            onChange={(e) => setComment(e.target.value)}
            rows={2}
            placeholder="На объект, на смену…"
            className="w-full rounded-xl border border-brand-100 bg-surface px-3 py-2.5 text-[15px] resize-none"
          />
        </div>
        <div>
          <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">Фото состояния при выдаче</label>
          <input
            type="file"
            accept="image/*"
            capture="environment"
            onChange={(e) => {
              const f = e.target.files?.[0]
              if (!f) return
              void preparePhoto(f).then((p) => setPhotoUrl(p.url))
            }}
          />
          {photoUrl && <img src={photoUrl} alt="" className="mt-2 h-20 rounded-lg object-cover" />}
        </div>
        <div className="flex justify-end gap-2">
          <SecondaryButton onClick={onClose}>Отмена</SecondaryButton>
          <PrimaryButton
            disabled={take.isPending || !qrLabel}
            onClick={() => {
              let dueAt: string | undefined
              if (!noDue) {
                const parsed = parseDueInput(dueLocal)
                if (!parsed.ok || !parsed.iso) {
                  window.alert('Укажите корректный срок возврата или отметьте «без срока»')
                  return
                }
                dueAt = parsed.iso
              }
              take.mutate({
                itemId: item.id,
                qrLabel,
                comment: comment || undefined,
                dueAt,
                photoUrl,
                quantity: qty > 1 || item.quantitative ? qty : undefined,
              })
            }}
          >
            {take.isPending ? <Loader2 size={16} className="animate-spin" /> : <PackageCheck size={16} />}
            Взять
          </PrimaryButton>
        </div>
      </div>
    </Modal>
  )
}

function FaultModal({
  open,
  onClose,
  item,
  onDone,
}: {
  open: boolean
  onClose: () => void
  item: ItemFull
  onDone: (msg: string) => void
}) {
  const utils = trpc.useUtils()
  const [description, setDescription] = useState('')
  const [severity, setSeverity] = useState<'low' | 'medium' | 'high'>('medium')
  const report = trpc.items.reportFault.useMutation({
    onSuccess: () => {
      utils.items.byId.invalidate({ id: item.id })
      utils.items.list.invalidate()
      utils.items.faults.invalidate()
      onDone('Неисправность зафиксирована, выдача заблокирована')
      onClose()
      setDescription('')
    },
    onError: (e) => onDone(e.message),
  })
  return (
    <Modal open={open} onClose={onClose} title="Сообщить о неисправности">
      <div className="space-y-4">
        <div>
          <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">Серьёзность</label>
          <select
            value={severity}
            onChange={(e) => setSeverity(e.target.value as 'low' | 'medium' | 'high')}
            className="w-full h-11 rounded-xl border border-brand-100 px-3"
          >
            <option value="low">Низкая</option>
            <option value="medium">Средняя</option>
            <option value="high">Высокая</option>
          </select>
        </div>
        <div>
          <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">Описание *</label>
          <textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            rows={3}
            placeholder="Что случилось с инструментом"
            className="w-full rounded-xl border border-brand-100 px-3 py-2.5 resize-none"
          />
        </div>
        <div className="flex justify-end gap-2">
          <SecondaryButton onClick={onClose}>Отмена</SecondaryButton>
          <PrimaryButton
            disabled={!description.trim() || report.isPending}
            onClick={() => report.mutate({ itemId: item.id, description: description.trim(), severity })}
          >
            {report.isPending ? <Loader2 size={16} className="animate-spin" /> : <AlertTriangle size={16} />}
            Отправить
          </PrimaryButton>
        </div>
      </div>
    </Modal>
  )
}

function ChangeRequestModal({
  open,
  onClose,
  item,
  onDone,
}: {
  open: boolean
  onClose: () => void
  item: ItemFull
  onDone: (msg: string) => void
}) {
  const utils = trpc.useUtils()
  const [title, setTitle] = useState(item.title)
  const [comment, setComment] = useState('')
  const req = trpc.items.requestChange.useMutation({
    onSuccess: () => {
      utils.items.changeRequests.invalidate()
      onDone('Заявка отправлена администратору')
      onClose()
    },
    onError: (e) => onDone(e.message),
  })
  return (
    <Modal open={open} onClose={onClose} title="Заявка на правку карточки">
      <div className="space-y-4">
        <div>
          <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">Предлагаемое название</label>
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            className="w-full h-11 rounded-xl border border-brand-100 px-3"
          />
        </div>
        <div>
          <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">Комментарий</label>
          <textarea
            value={comment}
            onChange={(e) => setComment(e.target.value)}
            rows={3}
            placeholder="Что именно исправить и почему"
            className="w-full rounded-xl border border-brand-100 px-3 py-2.5 resize-none"
          />
        </div>
        <div className="flex justify-end gap-2">
          <SecondaryButton onClick={onClose}>Отмена</SecondaryButton>
          <PrimaryButton
            disabled={req.isPending}
            onClick={() =>
              req.mutate({
                itemId: item.id,
                payload: { title: title.trim() },
                comment: comment || undefined,
              })
            }
          >
            {req.isPending ? <Loader2 size={16} className="animate-spin" /> : <Pencil size={16} />}
            Отправить заявку
          </PrimaryButton>
        </div>
      </div>
    </Modal>
  )
}

// ─── Секция 4. Карточка полей с inline-редактированием ───────────────────────

interface Draft {
  title: string
  categoryId: number | null
  brandId: number | null
  serialNumber: string
  cost: string
  buildingSiteId: number | null
  storageId: number | null
  responsibleUserId: number | null
  statusId: number | null
}

function draftFromItem(item: ItemFull): Draft {
  return {
    title: item.title,
    categoryId: item.categoryId ?? null,
    brandId: item.brandId ?? null,
    serialNumber: item.serialNumber ?? '',
    cost: item.cost == null ? '' : String(item.cost),
    buildingSiteId: item.buildingSiteId ?? null,
    storageId: item.storageId ?? null,
    responsibleUserId: item.responsibleUserId ?? null,
    statusId: item.statusId ?? null,
  }
}

function EditSelect({
  value,
  onChange,
  children,
}: {
  value: number | null
  onChange: (v: number | null) => void
  children: ReactNode
}) {
  return (
    <select
      value={value ?? ''}
      onChange={(e) => onChange(e.target.value ? Number(e.target.value) : null)}
      className="w-full h-10 rounded-xl border border-brand-100 bg-brand-50/60 px-2.5 text-[15px] text-ink-900 focus:border-brand-600 focus:ring-[3px] focus:ring-brand-600/15"
    >
      {children}
    </select>
  )
}

function EditInput({
  value,
  onChange,
  mono,
  placeholder,
}: {
  value: string
  onChange: (v: string) => void
  mono?: boolean
  placeholder?: string
}) {
  return (
    <input
      value={value}
      onChange={(e) => onChange(e.target.value)}
      placeholder={placeholder}
      className={cn(
        'w-full h-10 rounded-xl border border-brand-100 bg-brand-50/60 px-2.5 text-[15px] text-ink-900 placeholder:text-ink-300 focus:border-brand-600 focus:ring-[3px] focus:ring-brand-600/15',
        mono && 'font-mono-num'
      )}
    />
  )
}

function Field({
  label,
  children,
  flash,
}: {
  label: string
  children: ReactNode
  flash?: boolean
}) {
  return (
    <div
      className={cn(
        'rounded-xl px-2 py-1.5 -mx-2 transition-colors duration-700',
        flash && 'bg-success-bg'
      )}
    >
      <div className="text-caption text-ink-500 mb-0.5">{label}</div>
      <div className="text-[15px] leading-[22px] text-ink-900">{children}</div>
    </div>
  )
}

function FieldsCard({
  item,
  editing,
  draft,
  setDraft,
  flash,
  onReplenish,
  onWriteOff,
}: {
  item: ItemFull
  editing: boolean
  draft: Draft
  setDraft: (d: Draft) => void
  flash: boolean
  onReplenish: () => void
  onWriteOff: () => void
}) {
  const [renderedAt] = useState(Date.now)
  // Справочники нужны только в режиме редактирования
  const { data: categories } = trpc.admin.dictionaries.list.useQuery({ kind: 'categories' }, { enabled: editing })
  const { data: brands } = trpc.admin.dictionaries.list.useQuery({ kind: 'brands' }, { enabled: editing })
  const { data: statuses } = trpc.admin.dictionaries.list.useQuery({ kind: 'statuses' }, { enabled: editing })
  const { data: storages } = trpc.admin.storages.list.useQuery({}, { enabled: editing })
  const { data: sites } = trpc.admin.buildingSites.list.useQuery({}, { enabled: editing })
  const { data: users } = trpc.admin.users.list.useQuery({}, { enabled: editing })

  const metadata = item.metadata && typeof item.metadata === 'object'
    ? (item.metadata as Record<string, unknown>)
    : {}
  const metaText = (key: string) => {
    const value = metadata[key]
    if (Array.isArray(value)) return value.map(String).join(', ')
    if (typeof value === 'string' || typeof value === 'number') return String(value)
    if (typeof value === 'boolean') return value ? 'Да' : 'Нет'
    return ''
  }

  const set = <K extends keyof Draft>(key: K, value: Draft[K]) => setDraft({ ...draft, [key]: value })

  return (
    <motion.section
      initial={{ opacity: 0, y: 16 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.28, delay: 0.15, ease: 'easeOut' }}
      className="bg-surface rounded-card border border-brand-100/60 shadow-card p-5"
    >
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-x-4 gap-y-3">
        <Field label="Категория" flash={flash}>
          {editing ? (
            <EditSelect value={draft.categoryId} onChange={(v) => set('categoryId', v)}>
              <option value="">—</option>
              {(categories ?? []).map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </EditSelect>
          ) : (
            item.category?.name ?? '—'
          )}
        </Field>
        <Field label="Бренд" flash={flash}>
          {editing ? (
            <EditSelect value={draft.brandId} onChange={(v) => set('brandId', v)}>
              <option value="">—</option>
              {(brands ?? []).map((b) => (
                <option key={b.id} value={b.id}>
                  {b.name}
                </option>
              ))}
            </EditSelect>
          ) : (
            item.brand?.name ?? '—'
          )}
        </Field>
        <Field label="Вн. номер" flash={flash}>
          <span className="font-mono-num">{item.internalId}</span>
        </Field>
        <Field label="Серийный номер" flash={flash}>
          {editing ? (
            <EditInput mono value={draft.serialNumber} onChange={(v) => set('serialNumber', v)} placeholder="—" />
          ) : item.serialNumber ? (
            <span className="font-mono-num">{item.serialNumber}</span>
          ) : (
            '—'
          )}
        </Field>
        <Field label="Стоимость" flash={flash}>
          {editing ? (
            <EditInput value={draft.cost} onChange={(v) => set('cost', v.replace(/[^\d\s.,]/g, ''))} placeholder="0" />
          ) : (
            fmtMoney(item.cost)
          )}
        </Field>
        <Field label="Объект" flash={flash}>
          {editing ? (
            <EditSelect value={draft.buildingSiteId} onChange={(v) => set('buildingSiteId', v)}>
              <option value="">—</option>
              {(sites ?? []).map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </EditSelect>
          ) : item.buildingSite ? (
            <span className="inline-flex items-center gap-1.5">
              <Building2 size={14} strokeWidth={1.75} className="text-ink-300" />
              {item.buildingSite.name}
            </span>
          ) : (
            '—'
          )}
        </Field>
        <Field label="Склад" flash={flash}>
          {editing ? (
            <EditSelect value={draft.storageId} onChange={(v) => set('storageId', v)}>
              <option value="">—</option>
              {(storages ?? []).map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </EditSelect>
          ) : item.storage ? (
            <span className="inline-flex items-center gap-1.5">
              <Warehouse size={14} strokeWidth={1.75} className="text-ink-300" />
              {item.storage.name}
            </span>
          ) : (
            '—'
          )}
        </Field>
        <Field label="Ответственный" flash={flash}>
          {editing ? (
            <EditSelect value={draft.responsibleUserId} onChange={(v) => set('responsibleUserId', v)}>
              <option value="">—</option>
              {(users ?? [])
                .filter((u) => u.status === 'active')
                .map((u) => (
                  <option key={u.id} value={u.id}>
                    {u.fullName}
                  </option>
                ))}
            </EditSelect>
          ) : item.responsible ? (
            <span className="inline-flex items-center gap-2">
              {item.responsible.avatarUrl ? (
                <img
                  src={item.responsible.avatarUrl}
                  alt={item.responsible.fullName}
                  className="w-6 h-6 rounded-full object-cover border border-brand-100"
                />
              ) : (
                <span className="w-6 h-6 rounded-full bg-brand-100/60 flex items-center justify-center text-[11px] font-semibold text-brand-700">
                  {item.responsible.fullName.slice(0, 1)}
                </span>
              )}
              {item.responsible.fullName}
            </span>
          ) : (
            '—'
          )}
        </Field>
        <Field label="Статус" flash={flash}>
          {editing ? (
            <EditSelect value={draft.statusId} onChange={(v) => set('statusId', v)}>
              <option value="">—</option>
              {(statuses ?? []).map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </EditSelect>
          ) : (
            <ApiStatusBadge status={item.status} />
          )}
        </Field>
        <Field label="Срок возврата" flash={flash}>
          {item.dueAt ? (
            <span className="inline-flex items-center gap-1.5">
              <CalendarClock size={14} className="text-ink-300" />
              {fmtDateTime(item.dueAt)}
              {new Date(item.dueAt).getTime() < renderedAt && (
                <span className="text-[11px] font-semibold text-danger">просрочен</span>
              )}
            </span>
          ) : item.responsibleUserId ? (
            'Без срока'
          ) : (
            '—'
          )}
        </Field>
        <Field label="Учёт" flash={flash}>
          {item.quantitative ? (
            <span className="inline-flex items-center gap-1.5">
              <span className="inline-flex items-center gap-1 rounded-full bg-brand-100/60 px-2 py-0.5 text-[11px] font-semibold text-ink-500">
                Количественный
              </span>
              <span className="font-mono-num">
                склад {(item as { stockQty?: number }).stockQty ?? item.quantity ?? 0}
                {' · '}выдано {(item as { issuedQty?: number }).issuedQty ?? 0}
                {' · '}всего {(item as { totalQty?: number }).totalQty ?? item.quantity ?? 0} {item.unit ?? 'шт'}
              </span>
            </span>
          ) : (
            <span className="inline-flex items-center gap-1.5">
              <Wrench size={14} strokeWidth={1.75} className="text-ink-300" />
              Штучный
              {(item as { family?: { total?: number; inStock?: number; issued?: number } }).family &&
                Number((item as { family?: { total?: number } }).family?.total ?? 0) > 1 && (
                  <span className="font-mono-num text-ink-500">
                    · {(item as { family?: { inStock?: number } }).family?.inStock} на складе /{' '}
                    {(item as { family?: { issued?: number } }).family?.issued} выдано /{' '}
                    {(item as { family?: { total?: number } }).family?.total} шт
                  </span>
                )}
            </span>
          )}
        </Field>
        {(item.sourceSystem || item.externalId) && (
          <Field label="Источник данных" flash={flash}>
            {[item.sourceSystem, item.externalId].filter(Boolean).join(' · ')}
          </Field>
        )}
        {metaText('ownerCompany') && <Field label="Компания-владелец">{metaText('ownerCompany')}</Field>}
        {metaText('itemType') && <Field label="Тип ТМЦ">{metaText('itemType')}</Field>}
        {metaText('labels') && <Field label="Ярлыки">{metaText('labels')}</Field>}
        {metaText('depreciationUntil') && <Field label="Амортизация до">{metaText('depreciationUntil')}</Field>}
        {metaText('serviceLifeUntil') && <Field label="Срок службы до">{metaText('serviceLifeUntil')}</Field>}
        {metaText('rentalUntil') && <Field label="Аренда до">{metaText('rentalUntil')}</Field>}
        {metaText('supplier') && <Field label="Поставщик">{metaText('supplier')}</Field>}
        {(metaText('documentNumber') || metaText('documentDate')) && (
          <Field label="Документ">
            {[metaText('documentNumber'), metaText('documentDate')].filter(Boolean).join(' · ')}
          </Field>
        )}
        {metadata.isKit === true && <Field label="Комплект">Да</Field>}
      </div>

      {item.quantitative && !editing && (
        <div className="flex flex-wrap items-center gap-2 mt-4 pt-4 border-t border-brand-100/60">
          <span className="text-[13px] text-ink-500 mr-auto">
            Остаток: <span className="font-mono-num text-ink-900">{item.quantity ?? 0} {item.unit ?? 'шт'}</span>
          </span>
          <SecondaryButton onClick={onReplenish} className="!h-8 !px-3 !text-[13px]">
            <PlusCircle size={14} strokeWidth={1.75} className="text-success" />
            Пополнить
          </SecondaryButton>
          <SecondaryButton onClick={onWriteOff} className="!h-8 !px-3 !text-[13px]">
            <MinusCircle size={14} strokeWidth={1.75} className="text-danger" />
            Списать
          </SecondaryButton>
        </div>
      )}
    </motion.section>
  )
}

function HoldersBlock({ item }: { item: ItemFull }) {
  const holders = ((item as { holders?: Array<{ userId?: number; quantity?: number; internalId?: string; user?: { fullName?: string; avatarUrl?: string | null; phone?: string } | null }> }).holders ?? [])
    .filter((h) => h.user?.fullName)
  const family = (item as { family?: { total?: number; inStock?: number; issued?: number; members?: Array<{ id: number; internalId: string; inStock: boolean; responsible?: { fullName?: string } | null }> } }).family
  if (holders.length === 0 && !(family && Number(family.total) > 1)) return null
  return (
    <section className="bg-surface rounded-card border border-brand-100/60 shadow-card p-5 space-y-3">
      <h3 className="text-[17px] font-semibold text-ink-900">Где сейчас</h3>
      <div className="grid grid-cols-3 gap-2 text-center">
        <div className="rounded-xl bg-brand-50 px-2 py-2">
          <div className="font-mono-num text-lg font-semibold text-ink-900">{(item as { stockQty?: number }).stockQty ?? family?.inStock ?? 0}</div>
          <div className="text-[12px] text-ink-500">на складе</div>
        </div>
        <div className="rounded-xl bg-brand-50 px-2 py-2">
          <div className="font-mono-num text-lg font-semibold text-ink-900">{(item as { issuedQty?: number }).issuedQty ?? family?.issued ?? 0}</div>
          <div className="text-[12px] text-ink-500">выдано</div>
        </div>
        <div className="rounded-xl bg-brand-50 px-2 py-2">
          <div className="font-mono-num text-lg font-semibold text-ink-900">{(item as { totalQty?: number }).totalQty ?? family?.total ?? 1}</div>
          <div className="text-[12px] text-ink-500">всего</div>
        </div>
      </div>
      {holders.length > 0 ? (
        <ul className="space-y-2">
          {holders.map((h, i) => (
            <li key={`${h.userId}-${h.internalId ?? i}`} className="flex items-center gap-2 rounded-xl border border-brand-100/70 px-3 py-2">
              {h.user?.avatarUrl ? (
                <img src={h.user.avatarUrl} alt="" className="w-8 h-8 rounded-full object-cover" />
              ) : (
                <span className="w-8 h-8 rounded-full bg-brand-100/60 flex items-center justify-center text-xs font-semibold">
                  {(h.user?.fullName ?? '?').slice(0, 1)}
                </span>
              )}
              <span className="min-w-0 flex-1">
                <span className="block text-sm font-semibold truncate">{h.user?.fullName}</span>
                <span className="block text-[12px] text-ink-500">
                  {h.internalId ? h.internalId : `${h.quantity ?? 1} ${item.unit ?? 'шт'}`}
                </span>
              </span>
            </li>
          ))}
        </ul>
      ) : (
        <p className="text-sm text-ink-500">Все единицы на складе.</p>
      )}
    </section>
  )
}

// ─── Секция 5. Табы: История / Комментарии / Документы ───────────────────────

function HistoryTab({ item }: { item: ItemFull }) {
  const [openId, setOpenId] = useState<number | null>(null)
  const entries = [...item.history].sort(
    (a, b) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime()
  )

  if (entries.length === 0) {
    return <p className="text-sm text-ink-500 py-8 text-center">Операций пока не было.</p>
  }

  return (
    <div className="space-y-2">
      {entries.slice(0, 12).map((h, i) => {
        const meta = HISTORY_META[h.type] ?? HISTORY_META.update
        const Icon = meta.icon
        const open = openId === h.id
        const opId = `OP-${format(new Date(h.createdAt), 'yyyyMMdd')}-${String(h.id).padStart(3, '0')}`
        return (
          <motion.div
            key={h.id}
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.22, delay: Math.min(i, 11) * 0.03 }}
            className="rounded-xl border border-brand-100/60 bg-surface overflow-hidden"
          >
            <button
              onClick={() => setOpenId(open ? null : h.id)}
              className="w-full flex items-center gap-3 px-4 py-3 text-left hover:bg-brand-50/60 transition-colors"
            >
              <span
                className="w-8 h-8 shrink-0 rounded-full flex items-center justify-center"
                style={{ background: `${meta.color}1A`, color: meta.color }}
              >
                <Icon size={15} strokeWidth={1.75} />
              </span>
              <span className="font-mono-num text-ink-500 shrink-0 w-[130px]">{fmtDateTime(h.createdAt)}</span>
              <span className="flex-1 min-w-0 text-sm font-semibold text-ink-900 truncate">
                {meta.label}
                {(h.fromLabel || h.toLabel) && (
                  <span className="font-normal text-ink-500">
                    {' · '}
                    {[h.fromLabel, h.toLabel].filter(Boolean).join(' → ')}
                  </span>
                )}
              </span>
              {h.quantityDelta != null && h.quantityDelta !== 0 && (
                <span
                  className={cn(
                    'font-mono-num shrink-0',
                    h.quantityDelta > 0 ? 'text-success' : 'text-danger'
                  )}
                >
                  {h.quantityDelta > 0 ? '+' : ''}
                  {h.quantityDelta}
                </span>
              )}
              <ChevronDown
                size={16}
                strokeWidth={1.75}
                className={cn('shrink-0 text-ink-300 transition-transform duration-200', open && 'rotate-180')}
              />
            </button>
            <AnimatePresence initial={false}>
              {open && (
                <motion.div
                  initial={{ height: 0, opacity: 0 }}
                  animate={{ height: 'auto', opacity: 1 }}
                  exit={{ height: 0, opacity: 0 }}
                  transition={{ duration: 0.24, ease: 'easeOut' }}
                >
                  <div className="px-4 pb-4 pt-1 border-t border-brand-100/60 space-y-2 text-sm">
                    <div className="flex items-center gap-2 pt-2">
                      {h.actor?.avatarUrl ? (
                        <img src={h.actor.avatarUrl} alt="" className="w-6 h-6 rounded-full object-cover border border-brand-100" />
                      ) : (
                        <span className="w-6 h-6 rounded-full bg-brand-100/60 flex items-center justify-center text-[11px] font-semibold text-brand-700">
                          {h.actor?.fullName?.slice(0, 1) ?? '?'}
                        </span>
                      )}
                      <span className="text-ink-900 font-semibold">{h.actor?.fullName ?? 'Система'}</span>
                    </div>
                    {h.comment && <p className="text-ink-500">{h.comment}</p>}
                    <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                      <span className="font-mono-num text-ink-500">{opId}</span>
                      {h.opId && (
                        <span className="font-mono-num text-ink-300 text-[11px]">
                          {h.opId.slice(0, 10)}
                        </span>
                      )}
                    </div>
                  </div>
                </motion.div>
              )}
            </AnimatePresence>
          </motion.div>
        )
      })}
      {entries.length > 12 && (
        <p className="text-center text-[13px] text-ink-500 pt-1">
          Показаны последние 12 из {entries.length} — полный журнал на странице «История»
        </p>
      )}
    </div>
  )
}

function CommentsTab({ item, onDone }: { item: ItemFull; onDone: (msg: string) => void }) {
  const utils = trpc.useUtils()
  const [text, setText] = useState('')
  const listRef = useRef<HTMLDivElement>(null)
  const addComment = trpc.items.addComment.useMutation({
    onSuccess: () => {
      utils.items.byId.invalidate({ id: item.id })
      setText('')
      onDone('Комментарий добавлен')
      requestAnimationFrame(() => listRef.current?.scrollTo({ top: listRef.current.scrollHeight, behavior: 'smooth' }))
    },
  })

  const comments = [...item.comments].sort(
    (a, b) => new Date(a.createdAt).getTime() - new Date(b.createdAt).getTime()
  )

  return (
    <div className="flex flex-col gap-4">
      <div ref={listRef} className="space-y-3 max-h-[420px] overflow-y-auto pr-1">
        {comments.length === 0 && (
          <p className="text-sm text-ink-500 py-6 text-center">
            Комментариев пока нет — начните обсуждение.
          </p>
        )}
        {comments.map((c) => (
          <motion.div
            key={c.id}
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.2 }}
            className="flex gap-3"
          >
            {c.user?.avatarUrl ? (
              <img src={c.user.avatarUrl} alt="" className="w-8 h-8 shrink-0 rounded-full object-cover border border-brand-100" />
            ) : (
              <span className="w-8 h-8 shrink-0 rounded-full bg-brand-100/60 flex items-center justify-center text-xs font-semibold text-brand-700">
                {c.user?.fullName?.slice(0, 1) ?? '?'}
              </span>
            )}
            <div className="min-w-0">
              <div className="flex flex-wrap items-baseline gap-x-2">
                <span className="text-sm font-semibold text-ink-900">{c.user?.fullName ?? 'Пользователь'}</span>
                <span className="text-[13px] text-ink-500">{fmtDateTime(c.createdAt)}</span>
              </div>
              <div className="mt-1 inline-block rounded-xl bg-brand-50 px-3 py-2 text-[15px] leading-[22px] text-ink-900 whitespace-pre-wrap break-words">
                {c.text}
              </div>
            </div>
          </motion.div>
        ))}
      </div>
      <form
        onSubmit={(e) => {
          e.preventDefault()
          if (text.trim()) addComment.mutate({ itemId: item.id, text: text.trim() })
        }}
        className="flex items-end gap-2 border-t border-brand-100/60 pt-3"
      >
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          rows={Math.min(5, Math.max(1, text.split('\n').length))}
          placeholder="Написать комментарий…"
          className="flex-1 rounded-xl border border-brand-100 bg-surface px-3 py-2.5 text-[15px] text-ink-900 placeholder:text-ink-300 focus:border-brand-600 focus:ring-[3px] focus:ring-brand-600/15 resize-none max-h-[120px]"
        />
        <PrimaryButton type="submit" disabled={!text.trim() || addComment.isPending} className="!h-10">
          {addComment.isPending ? <Loader2 size={16} className="animate-spin" /> : <Send size={16} strokeWidth={1.75} />}
          Отправить
        </PrimaryButton>
      </form>
    </div>
  )
}

function DocumentsTab({ item, onDone }: { item: ItemFull; onDone: (msg: string) => void }) {
  const docs = item.documents ?? []
  const { currentUser } = useStore()
  const utils = trpc.useUtils()
  const fileRef = useRef<HTMLInputElement>(null)
  const [accessLevel, setAccessLevel] = useState<'members' | 'accounting' | 'managers'>('members')
  const [uploading, setUploading] = useState(false)
  const ingestContent = trpc.content.ingest.useMutation()
  const addDocument = trpc.items.addDocument.useMutation()
  const canManage = currentUser?.roleRights.manageDocuments === true

  const upload = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    event.target.value = ''
    if (!file || uploading) return
    if (file.size === 0 || file.size > BROWSER_FILE_LIMIT_BYTES) {
      onDone(`Документ должен занимать от 1 байта до ${BROWSER_FILE_LIMIT_LABEL}`)
      return
    }
    setUploading(true)
    try {
      if (!item.guid) throw new Error('Карточка ещё не получила переносимый GUID')
      const dataUrl = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader()
        reader.onload = () => resolve(String(reader.result))
        reader.onerror = () => reject(reader.error ?? new Error('Не удалось прочитать документ'))
        reader.readAsDataURL(file)
      })
      const uploaded = await ingestContent.mutateAsync({
        workspaceId: item.workspaceId,
        itemId: item.id,
        purpose: 'item-document',
        dataUrl,
      })
      await addDocument.mutateAsync({
        itemId: item.id,
        itemGuid: item.guid,
        documentGuid: crypto.randomUUID(),
        name: file.name.slice(0, 200),
        url: uploaded.url,
        mime: uploaded.mime,
        accessLevel,
      })
      await utils.items.byId.invalidate({ id: item.id })
      onDone('Документ сохранён в CAS и подписан')
    } catch (error) {
      onDone(error instanceof Error ? error.message : 'Не удалось загрузить документ')
    } finally {
      setUploading(false)
    }
  }

  return (
    <div className="space-y-4">
      {docs.length > 0 ? (
        <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-3">
          {docs.map((d) => {
            const isImage = /\.(png|jpe?g|webp|gif)$/i.test(d.url)
            const Icon = isImage ? FileImage : FileText
            return (
              <div
                key={d.id}
                className="group flex items-center gap-3 rounded-xl border border-brand-100/60 bg-surface p-3 hover:shadow-card transition-shadow"
              >
                <span className="w-10 h-10 shrink-0 rounded-xl bg-brand-50 flex items-center justify-center text-brand-600">
                  <Icon size={18} strokeWidth={1.75} />
                </span>
                <div className="min-w-0 flex-1">
                  <div className="text-sm font-semibold text-ink-900 truncate">{d.name}</div>
                  <a
                    href={d.url}
                    download={d.name}
                    className="text-[13px] font-semibold text-brand-600 hover:text-brand-700"
                  >
                    Скачать
                  </a>
                </div>
              </div>
            )
          })}
        </div>
      ) : (
        <p className="text-sm text-ink-500 py-4 text-center">Документов пока нет.</p>
      )}
      {canManage && (
        <div className="rounded-xl border border-dashed border-brand-100 p-4 space-y-3">
          <label className="block text-[13px] font-semibold text-ink-500">
            Кто увидит документ
            <select
              aria-label="Доступ к документу"
              value={accessLevel}
              onChange={(event) => setAccessLevel(event.target.value as typeof accessLevel)}
              className="mt-1.5 h-10 w-full rounded-xl border border-brand-100 bg-surface px-3 text-sm text-ink-900"
            >
              <option value="members">Все участники с доступом к документам</option>
              <option value="accounting">Только бухгалтерия</option>
              <option value="managers">Только управляющие документами</option>
            </select>
          </label>
          <input
            ref={fileRef}
            data-testid="tool-document-file"
            type="file"
            className="hidden"
            onChange={upload}
          />
          <button
            type="button"
            onClick={() => fileRef.current?.click()}
            disabled={uploading}
            className="w-full rounded-xl border border-brand-100 px-4 py-3 text-sm font-semibold text-ink-500 hover:bg-brand-50 transition-colors flex items-center justify-center gap-2 disabled:opacity-60"
          >
            {uploading ? <Loader2 size={16} className="animate-spin" /> : <Upload size={16} strokeWidth={1.75} />}
            {uploading ? 'Сохраняем и подписываем…' : `Загрузить документ (до ${BROWSER_FILE_LIMIT_LABEL})`}
          </button>
        </div>
      )}
    </div>
  )
}

// ─── Секция 6. Похожие инструменты ───────────────────────────────────────────

function SimilarTools({ item }: { item: ItemFull }) {
  const { data } = trpc.items.list.useQuery(
    { categoryId: item.categoryId ?? undefined, limit: 7 },
    { enabled: item.categoryId != null }
  )
  const similar = (data?.rows ?? []).filter((t) => t.id !== item.id).slice(0, 6)
  if (similar.length === 0) return null

  return (
    <section className="mt-8">
      <h2 className="text-xl leading-7 font-semibold text-ink-900 mb-4">Похожие инструменты</h2>
      <div className="flex gap-3 overflow-x-auto pb-2 -mx-1 px-1">
        {similar.map((t, i) => {
          const photo = t.photos.find((p) => p.isTitle) ?? t.photos[0]
          return (
            <motion.div
              key={t.id}
              initial={{ opacity: 0, y: 20 }}
              whileInView={{ opacity: 1, y: 0 }}
              viewport={{ once: true, amount: 0.2 }}
              transition={{ duration: 0.26, delay: Math.min(i, 5) * 0.05 }}
              className="w-[180px] shrink-0"
            >
              <Link
                to={`/tool/${t.id}`}
                className="block bg-surface rounded-mini border border-brand-100/60 shadow-card p-2.5 transition-all duration-200 hover:shadow-hover hover:-translate-y-0.5"
              >
                <div className="aspect-[4/3] rounded-[10px] overflow-hidden bg-brand-50">
                  {photo ? (
                    <img src={photo.url} alt={t.title} className="w-full h-full object-cover" loading="lazy" />
                  ) : (
                    <div className="w-full h-full flex items-center justify-center text-ink-300">
                      <Wrench size={24} strokeWidth={1.75} />
                    </div>
                  )}
                </div>
                <div className="pt-2">
                  <span className="font-mono-num text-ink-500">{t.internalId}</span>
                  <div className="text-[13px] leading-[18px] font-semibold text-ink-900 line-clamp-2 min-h-[36px] mt-0.5">
                    {t.title}
                  </div>
                  <div className="mt-1">
                    <ApiStatusBadge status={t.status} />
                  </div>
                </div>
              </Link>
            </motion.div>
          )
        })}
      </div>
    </section>
  )
}

// ─── Главный компонент страницы ───────────────────────────────────────────────

export default function ToolCard() {
  const { id } = useParams()
  const navigate = useNavigate()
  const itemId = Number(id)
  const utils = trpc.useUtils()
  const { toast, showToast } = useToast()
  const { currentUser } = useStore()
  const meId = currentUser?.id

  const returnMut = trpc.transfers.returnItem.useMutation({
    onSuccess: () => {
      utils.items.byId.invalidate({ id: itemId })
      utils.items.list.invalidate()
      utils.meta.transferCounts.invalidate()
      showToast('Инструмент возвращён на склад')
    },
    onError: (e) => showToast(e.message),
  })

  const { data: item, isLoading, isError } = trpc.items.byId.useQuery(
    { id: itemId },
    { enabled: Number.isFinite(itemId) && itemId > 0 }
  )

  const [localPhotos, setLocalPhotos] = useState<string[]>([])
  const [editing, setEditing] = useState(false)
  const [draft, setDraft] = useState<Draft | null>(null)
  const [flash, setFlash] = useState(false)
  const [tab, setTab] = useState<'history' | 'comments' | 'docs'>('history')
  const [transferOpen, setTransferOpen] = useState(false)
  const [takeOpen, setTakeOpen] = useState(false)
  const [faultOpen, setFaultOpen] = useState(false)
  const [changeOpen, setChangeOpen] = useState(false)
  const [qrOpen, setQrOpen] = useState(false)
  const [deleteOpen, setDeleteOpen] = useState(false)
  const [qtyMode, setQtyMode] = useState<'replenish' | 'writeOff' | null>(null)

  // Нужен слаг статуса, чтобы понять, обязательна ли причина смены (ТЗ §8).
  const { data: statuses } = trpc.admin.dictionaries.list.useQuery(
    { kind: 'statuses' },
    { enabled: editing },
  )

  const update = trpc.items.update.useMutation({
    onSuccess: () => {
      utils.items.byId.invalidate({ id: itemId })
      utils.items.list.invalidate()
      setEditing(false)
      setFlash(true)
      showToast('Изменения сохранены')
      setTimeout(() => setFlash(false), 900)
    },
  })

  const photos = useMemo(() => {
    const base = (item?.photos ?? []).map((p) => p.url)
    return [...base, ...localPhotos]
  }, [item, localPhotos])

  // Дата последнего получения ответственным (для подписи статуса)
  const responsibleSince = useMemo(() => {
    if (!item) return null
    const last = [...item.history]
      .filter((h) => h.type === 'transfer_receive' || h.type === 'move' || h.type === 'create')
      .sort((a, b) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime())[0]
    return last ? new Date(last.createdAt) : new Date(item.createdAt)
  }, [item])

  if (isLoading) {
    return (
      <div className="space-y-4">
        <div className="h-4 w-64 rounded animate-skeleton-pulse" />
        <div className="grid lg:grid-cols-[55%_1fr] gap-6">
          <div className="aspect-[4/3] rounded-card animate-skeleton-pulse" />
          <div className="space-y-3">
            <div className="h-8 w-3/4 rounded animate-skeleton-pulse" />
            <div className="h-64 rounded-card animate-skeleton-pulse" />
          </div>
        </div>
      </div>
    )
  }

  if (isError || !item) {
    return (
      <div className="bg-surface rounded-card border border-brand-100/60 shadow-card p-10 text-center space-y-3">
        <h1 className="text-xl font-semibold text-ink-900">Инструмент не найден</h1>
        <p className="text-sm text-ink-500">Карточка с таким номером удалена или не существует.</p>
        <SecondaryButton onClick={() => navigate('/')}>
          <ArrowLeft size={16} strokeWidth={1.75} />
          Назад к каталогу
        </SecondaryButton>
      </div>
    )
  }

  const copyInternalId = async () => {
    try {
      await navigator.clipboard.writeText(item.internalId)
      showToast('Номер скопирован')
    } catch {
      showToast(`Номер: ${item.internalId}`)
    }
  }

  const startEdit = () => {
    setDraft(draftFromItem(item))
    setEditing(true)
  }

  const saveEdit = () => {
    if (!draft) return
    if (!draft.title.trim()) {
      showToast('Наименование не может быть пустым')
      return
    }
    let reason: string | undefined
    if (draft.statusId !== item.statusId) {
      const next = (statuses ?? []).find((s) => s.id === draft.statusId)
      if (statusNeedsReason(next?.slug)) {
        const answer = askStatusReason(next?.name ?? '')
        if (!answer) {
          showToast('Нужна причина смены статуса (минимум 3 символа)')
          return
        }
        reason = answer
      }
    }
    update.mutate({
      reason,
      id: item.id,
      title: draft.title.trim(),
      categoryId: draft.categoryId,
      brandId: draft.brandId,
      serialNumber: draft.serialNumber.trim() || null,
      cost: draft.cost.trim() ? Number(draft.cost.replace(/\s/g, '').replace(',', '.')) : null,
      buildingSiteId: draft.buildingSiteId,
      storageId: draft.storageId,
      responsibleUserId: draft.responsibleUserId,
      statusId: draft.statusId,
    })
  }

  const tabs = [
    { key: 'history' as const, label: 'История', count: item.history.length, icon: HistoryIcon },
    { key: 'comments' as const, label: 'Комментарии', count: item.comments.length, icon: MessageSquare },
    { key: 'docs' as const, label: 'Документы', count: item.documents.length, icon: FileText },
  ]

  const actions = (
    <>
      {editing ? (
        <>
          <PrimaryButton onClick={saveEdit} disabled={update.isPending} className="flex-1 lg:flex-none">
            {update.isPending ? <Loader2 size={16} className="animate-spin" /> : <CheckCircle2 size={16} strokeWidth={1.75} />}
            Сохранить
          </PrimaryButton>
          <SecondaryButton
            onClick={() => {
              setEditing(false)
              setDraft(null)
            }}
          >
            Отмена
          </SecondaryButton>
        </>
      ) : (
        <>
          {itemCirculates(item.status?.slug) && (
            (item.quantitative
              ? Number((item as { stockQty?: number }).stockQty ?? item.quantity ?? 0) > 0
              : item.responsibleUserId == null || Number((item as { family?: { inStock?: number } }).family?.inStock ?? 0) > 0) && (
            <PrimaryButton
              onClick={() => setTakeOpen(true)}
              className="flex-1 lg:flex-none"
            >
              <PackageCheck size={16} strokeWidth={1.75} />
              Взять
            </PrimaryButton>
            )
          )}
          {(item.responsibleUserId === meId ||
            (Array.isArray((item as { holders?: { userId?: number }[] }).holders) &&
              (item as { holders?: { userId?: number }[] }).holders!.some((h) => h.userId === meId))) && (
            <PrimaryButton
              onClick={() => returnMut.mutate({ itemId: item.id })}
              disabled={returnMut.isPending}
              className="flex-1 lg:flex-none"
            >
              {returnMut.isPending ? <Loader2 size={16} className="animate-spin" /> : <Undo2 size={16} strokeWidth={1.75} />}
              Вернуть
            </PrimaryButton>
          )}
          {itemCirculates(item.status?.slug) && (
            <PrimaryButton onClick={() => setTransferOpen(true)} className="flex-1 lg:flex-none">
              <ArrowLeftRight size={16} strokeWidth={1.75} />
              Передать
            </PrimaryButton>
          )}
          <SecondaryButton onClick={startEdit}>
            <Pencil size={16} strokeWidth={1.75} />
            Редактировать
          </SecondaryButton>
          <SecondaryButton onClick={() => setChangeOpen(true)}>
            Заявка на правку
          </SecondaryButton>
          <SecondaryButton onClick={() => setFaultOpen(true)}>
            <AlertTriangle size={16} strokeWidth={1.75} />
            Неисправность
          </SecondaryButton>
          <IconButton onClick={() => setQrOpen(true)} aria-label="Печать QR" title="Печать QR">
            <QrCode size={18} strokeWidth={1.75} />
          </IconButton>
          <IconButton danger onClick={() => setDeleteOpen(true)} aria-label="Удалить" title="Удалить">
            <Trash2 size={18} strokeWidth={1.75} />
          </IconButton>
        </>
      )}
    </>
  )

  return (
    <div className="space-y-5">
      {/* Секция 1. Шапка: хлебные крошки */}
      <motion.div
        initial={{ opacity: 0, y: -8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.2 }}
        className="flex items-center justify-between gap-3"
      >
        <nav className="text-[13px] text-ink-500 flex flex-wrap items-center gap-x-1.5 min-w-0">
          <Link to="/" className="hover:text-brand-600 transition-colors">
            Все ТМЦ
          </Link>
          <span>/</span>
          {item.category && (
            <>
              <span className="truncate">{item.category.name}</span>
              <span>/</span>
            </>
          )}
          <span className="font-mono-num text-ink-900">{item.internalId}</span>
        </nav>
        <button
          onClick={() => navigate('/')}
          className="lg:hidden shrink-0 inline-flex items-center gap-1.5 text-[13px] font-semibold text-brand-600 hover:bg-brand-50 rounded-lg px-2 py-1.5 transition-colors"
        >
          <ArrowLeft size={14} strokeWidth={1.75} />
          Назад к каталогу
        </button>
      </motion.div>

      {/* Две колонки */}
      <div className="grid lg:grid-cols-[55%_1fr] gap-6 items-start">
        <PhotoGallery
          photos={photos}
          status={item.status}
          hasQr={item.qrCode != null}
          onAddLocal={(url) => setLocalPhotos((p) => [...p, url])}
        />

        {/* Правая колонка */}
        <div className="space-y-4">
          <motion.div
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.24, delay: 0.05 }}
            className="lg:sticky lg:top-20 space-y-4"
          >
            <div>
              <div className="flex items-center gap-1.5">
                <span className="font-mono-num text-ink-500">{item.internalId}</span>
                <button
                  onClick={copyInternalId}
                  aria-label="Скопировать номер"
                  className="w-7 h-7 inline-flex items-center justify-center rounded-lg text-ink-300 hover:text-brand-600 hover:bg-brand-50 transition-colors"
                >
                  <Copy size={13} strokeWidth={1.75} />
                </button>
              </div>
              {editing && draft ? (
                <input
                  value={draft.title}
                  onChange={(e) => setDraft({ ...draft, title: e.target.value })}
                  className="mt-1 w-full h-12 rounded-xl border border-brand-100 bg-brand-50/60 px-3 text-2xl lg:text-[28px] leading-9 font-bold tracking-[-0.01em] text-ink-900 focus:border-brand-600 focus:ring-[3px] focus:ring-brand-600/15"
                />
              ) : (
                <h1 className="mt-1 text-2xl lg:text-[28px] leading-9 font-bold tracking-[-0.01em] text-ink-900">
                  {item.title}
                </h1>
              )}
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <ApiStatusBadge status={item.status} />
                {item.responsible && responsibleSince && (
                  <span className="text-[13px] text-ink-500">
                    на ответственном хранении у {item.responsible.fullName} с {fmtDate(responsibleSince)}
                  </span>
                )}
              </div>
            </div>

            {/* Панель действий (десктоп) */}
            <motion.div
              initial={{ opacity: 0, y: 12 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.24, delay: 0.1 }}
              className="hidden lg:flex items-center gap-2 flex-wrap"
            >
              {actions}
            </motion.div>
          </motion.div>

          <FieldsCard
            item={item}
            editing={editing}
            draft={draft ?? draftFromItem(item)}
            setDraft={setDraft}
            flash={flash}
            onReplenish={() => setQtyMode('replenish')}
            onWriteOff={() => setQtyMode('writeOff')}
          />

          <HoldersBlock item={item} />
        </div>
      </div>

      {/* Секция 5. Табы */}
      <motion.section
        initial={{ opacity: 0, y: 16 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.28, delay: 0.2 }}
        className="bg-surface rounded-card border border-brand-100/60 shadow-card p-4 sm:p-5"
      >
        <div className="flex gap-5 border-b border-brand-100/60 overflow-x-auto">
          {tabs.map((t) => (
            <button
              key={t.key}
              onClick={() => setTab(t.key)}
              className={cn(
                'relative flex items-center gap-1.5 pb-3 text-sm font-semibold whitespace-nowrap transition-colors',
                tab === t.key ? 'text-brand-600' : 'text-ink-500 hover:text-ink-900'
              )}
            >
              <t.icon size={15} strokeWidth={1.75} />
              {t.label}
              <span className={cn('font-mono-num', tab === t.key ? 'text-accent' : 'text-ink-500')}>
                ({t.count})
              </span>
              {tab === t.key && (
                <motion.span
                  layoutId="toolcard-tab-underline"
                  className="absolute -bottom-px left-0 right-0 h-0.5 bg-accent rounded-full"
                  transition={{ duration: 0.2 }}
                />
              )}
            </button>
          ))}
        </div>
        <AnimatePresence mode="wait">
          <motion.div
            key={tab}
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.18 }}
            className="pt-4"
          >
            {tab === 'history' && <HistoryTab item={item} />}
            {tab === 'comments' && <CommentsTab item={item} onDone={showToast} />}
            {tab === 'docs' && <DocumentsTab item={item} onDone={showToast} />}
          </motion.div>
        </AnimatePresence>
      </motion.section>

      <SimilarTools item={item} />

      {/* Мобайл: фиксированная панель действий */}
      <div className="lg:hidden fixed bottom-16 left-0 right-0 z-40 px-3 pb-2 pointer-events-none">
        <div className="pointer-events-auto bg-surface rounded-card shadow-modal border border-brand-100/60 p-2 flex items-center gap-2">
          {actions}
        </div>
      </div>

      {/* Модалки */}
      <TransferModal open={transferOpen} onClose={() => setTransferOpen(false)} item={item} onDone={showToast} />
      <TakeModal open={takeOpen} onClose={() => setTakeOpen(false)} item={item} onDone={showToast} />
      <FaultModal open={faultOpen} onClose={() => setFaultOpen(false)} item={item} onDone={showToast} />
      <ChangeRequestModal open={changeOpen} onClose={() => setChangeOpen(false)} item={item} onDone={showToast} />
      <QrModal open={qrOpen} onClose={() => setQrOpen(false)} item={item} />
      <DeleteModal
        open={deleteOpen}
        onClose={() => setDeleteOpen(false)}
        item={item}
        onDeleted={() => {
          showToast('Карточка удалена')
          navigate('/')
        }}
      />
      {qtyMode && (
        <QuantityModal
          open
          onClose={() => setQtyMode(null)}
          item={item}
          mode={qtyMode}
          onDone={showToast}
        />
      )}

      <Toast text={toast} />
    </div>
  )
}
