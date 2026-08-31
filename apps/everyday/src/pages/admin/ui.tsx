import { createContext, useCallback, useContext, useState } from 'react'
import type { ReactNode } from 'react'
import { AnimatePresence, motion } from 'framer-motion'
import { CheckCircle2, Info, TriangleAlert, X } from 'lucide-react'
import { cn } from '@/lib/utils'

/* ─── Общие классы (design.md §6) ─────────────────────────────────────────── */

export const cardCls = 'bg-surface rounded-card border border-brand-100/60 shadow-card'
export const inputCls =
  'h-11 w-full rounded-xl border border-brand-100 bg-surface px-4 text-[15px] text-ink-900 placeholder:text-ink-300 outline-none transition focus:border-brand-600 focus:ring-[3px] focus:ring-[#5E629B22] disabled:bg-brand-50 disabled:text-ink-300'
export const btnPrimaryCls =
  'inline-flex h-10 items-center justify-center gap-2 rounded-xl bg-accent px-5 text-sm font-semibold text-white transition hover:-translate-y-px hover:bg-accent-hover active:scale-[0.97] disabled:pointer-events-none disabled:opacity-50'
export const btnSecondaryCls =
  'inline-flex h-10 items-center justify-center gap-2 rounded-xl border border-brand-100 bg-surface px-5 text-sm font-semibold text-ink-900 transition hover:bg-brand-50 active:scale-[0.97] disabled:pointer-events-none disabled:opacity-50'
export const btnGhostCls =
  'inline-flex h-10 items-center justify-center gap-2 rounded-xl px-3 text-sm font-semibold text-brand-600 transition hover:bg-brand-50'
export const thCls =
  'bg-brand-50 px-4 py-3 text-left text-caption text-ink-500 first:rounded-l-xl last:rounded-r-xl whitespace-nowrap'
export const tdCls = 'border-b border-brand-100/70 px-4 py-3 align-middle text-sm'

/* ─── Утилиты ─────────────────────────────────────────────────────────────── */

export function plural(n: number, forms: [string, string, string]): string {
  const m = Math.abs(n) % 100
  const d = m % 10
  if (m > 10 && m < 20) return forms[2]
  if (d > 1 && d < 5) return forms[1]
  if (d === 1) return forms[0]
  return forms[2]
}

/** Маска российского телефона: +7 XXX XXX-XX-XX */
export function formatPhoneInput(raw: string): string {
  let d = raw.replace(/\D/g, '')
  if (d.startsWith('8')) d = '7' + d.slice(1)
  if (d && !d.startsWith('7')) d = '7' + d
  d = d.slice(0, 11)
  const p = d.slice(1)
  let out = p.length ? '+7' : d ? '+7' : ''
  if (p.length > 0) out += ' ' + p.slice(0, 3)
  if (p.length > 3) out += ' ' + p.slice(3, 6)
  if (p.length > 6) out += '-' + p.slice(6, 8)
  if (p.length > 8) out += '-' + p.slice(8, 10)
  return out
}

export function fmtDate(d: Date | string | null | undefined): string {
  if (!d) return '—'
  const date = d instanceof Date ? d : new Date(d)
  if (Number.isNaN(date.getTime())) return '—'
  const dd = String(date.getDate()).padStart(2, '0')
  const mm = String(date.getMonth() + 1).padStart(2, '0')
  return `${dd}.${mm}.${date.getFullYear()}`
}

/* ─── Тосты (design.md §6: тёмный pill снизу по центру) ───────────────────── */

type ToastType = 'success' | 'info' | 'error'
interface ToastItem {
  id: number
  message: string
  type: ToastType
}

const ToastCtx = createContext<(message: string, type?: ToastType) => void>(() => {})

export function useToast() {
  return useContext(ToastCtx)
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<ToastItem[]>([])

  const push = useCallback((message: string, type: ToastType = 'success') => {
    const id = Date.now() + Math.random()
    setToasts((t) => [...t, { id, message, type }])
    window.setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), 4000)
  }, [])

  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div className="pointer-events-none fixed inset-x-0 bottom-20 lg:bottom-6 z-[90] flex flex-col items-center gap-2 px-4">
        <AnimatePresence>
          {toasts.map((t) => (
            <motion.div
              key={t.id}
              initial={{ opacity: 0, y: 24 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: 12 }}
              transition={{ duration: 0.22, ease: [0.22, 1, 0.36, 1] }}
              className="pointer-events-auto inline-flex items-center gap-2 rounded-full bg-ink-900 px-4 py-2.5 text-sm font-semibold text-white shadow-modal"
            >
              {t.type === 'success' && <CheckCircle2 size={16} className="text-teal" />}
              {t.type === 'info' && <Info size={16} className="text-teal" />}
              {t.type === 'error' && <TriangleAlert size={16} className="text-danger" />}
              {t.message}
            </motion.div>
          ))}
        </AnimatePresence>
      </div>
    </ToastCtx.Provider>
  )
}

/* ─── Модалка (design.md §6: spring scale, overlay + blur) ────────────────── */

export function Modal({
  open,
  onClose,
  title,
  children,
  maxWidth = 560,
}: {
  open: boolean
  onClose: () => void
  title: string
  children: ReactNode
  maxWidth?: number
}) {
  return (
    <AnimatePresence>
      {open && (
        <div className="fixed inset-0 z-[80] flex items-center justify-center p-3 sm:p-4">
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.18 }}
            onClick={onClose}
            className="absolute inset-0 bg-[rgba(48,52,102,.45)] backdrop-blur-[4px]"
          />
          <motion.div
            initial={{ opacity: 0, scale: 0.96 }}
            animate={{ opacity: 1, scale: 1 }}
            exit={{ opacity: 0, scale: 0.96 }}
            transition={{ type: 'spring', stiffness: 300, damping: 26 }}
            className="relative flex max-h-[88vh] w-full flex-col rounded-modal bg-surface shadow-modal"
            style={{ maxWidth }}
          >
            <div className="flex items-center justify-between gap-4 border-b border-brand-100/70 px-5 py-4 sm:px-6">
              <h3 className="text-[17px] leading-6 font-semibold text-ink-900">{title}</h3>
              <button
                type="button"
                onClick={onClose}
                className="flex h-8 w-8 items-center justify-center rounded-xl text-ink-500 transition hover:bg-brand-50 hover:text-ink-900"
                aria-label="Закрыть"
              >
                <X size={18} />
              </button>
            </div>
            <div className="overflow-y-auto px-5 py-5 sm:px-6">{children}</div>
          </motion.div>
        </div>
      )}
    </AnimatePresence>
  )
}

/* ─── Подтверждение опасного действия ─────────────────────────────────────── */

export function ConfirmModal({
  open,
  onClose,
  onConfirm,
  title,
  text,
  confirmLabel = 'Удалить',
  loading = false,
}: {
  open: boolean
  onClose: () => void
  onConfirm: () => void
  title: string
  text: string
  confirmLabel?: string
  loading?: boolean
}) {
  return (
    <Modal open={open} onClose={onClose} title={title} maxWidth={440}>
      <p className="text-sm leading-[22px] text-ink-500">{text}</p>
      <div className="mt-5 flex justify-end gap-2">
        <button type="button" className={btnSecondaryCls} onClick={onClose}>
          Отмена
        </button>
        <button
          type="button"
          onClick={onConfirm}
          disabled={loading}
          className="inline-flex h-10 items-center justify-center gap-2 rounded-xl bg-danger px-5 text-sm font-semibold text-white transition hover:-translate-y-px hover:brightness-95 active:scale-[0.97] disabled:pointer-events-none disabled:opacity-50"
        >
          {loading ? 'Подождите…' : confirmLabel}
        </button>
      </div>
    </Modal>
  )
}

/* ─── Тоггл (admin.md: track 40×22 brand-100, on — brand-600, thumb 18px) ─── */

export function Toggle({
  checked,
  onChange,
  disabled = false,
}: {
  checked: boolean
  onChange: (v: boolean) => void
  disabled?: boolean
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation()
        onChange(!checked)
      }}
      className={cn(
        'relative h-[22px] w-10 shrink-0 rounded-full transition-colors duration-200',
        checked ? 'bg-brand-600' : 'bg-brand-100',
        disabled && 'pointer-events-none opacity-40'
      )}
    >
      <motion.span
        animate={{ x: checked ? 18 : 0 }}
        transition={{ type: 'spring', stiffness: 500, damping: 32 }}
        className="absolute left-[2px] top-[2px] block h-[18px] w-[18px] rounded-full bg-white shadow"
      />
    </button>
  )
}

/* ─── Поле формы ──────────────────────────────────────────────────────────── */

export function Field({
  label,
  required = false,
  children,
}: {
  label: string
  required?: boolean
  children: ReactNode
}) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-[13px] leading-[18px] font-semibold text-ink-500">
        {label}
        {required && <span className="text-accent"> *</span>}
      </span>
      {children}
    </label>
  )
}

/* ─── Аватар с fallback-инициалами ────────────────────────────────────────── */

export function UserAvatar({
  name,
  url,
  size = 32,
}: {
  name: string
  url?: string | null
  size?: number
}) {
  const initials = name
    .split(' ')
    .map((w) => w[0])
    .filter(Boolean)
    .slice(0, 2)
    .join('')
    .toUpperCase()
  const style = { width: size, height: size }
  if (url) {
    return (
      <img
        src={url}
        alt={name}
        style={style}
        className="shrink-0 rounded-full border border-brand-100 object-cover"
      />
    )
  }
  return (
    <span
      style={{ ...style, fontSize: size * 0.36 }}
      className="flex shrink-0 items-center justify-center rounded-full bg-brand-100 font-semibold text-brand-700"
    >
      {initials}
    </span>
  )
}

/* ─── Шапка раздела ───────────────────────────────────────────────────────── */

export function SectionHeader({
  title,
  count,
  action,
}: {
  title: string
  count?: number
  action?: ReactNode
}) {
  return (
    <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
      <h2 className="flex items-center gap-2 text-xl leading-7 font-semibold text-ink-900">
        {title}
        {typeof count === 'number' && (
          <span className="font-mono-num text-ink-500">· {count}</span>
        )}
      </h2>
      {action}
    </div>
  )
}

/* ─── Строка-заглушка таблицы ─────────────────────────────────────────────── */

export function TablePlaceholder({ colSpan, text }: { colSpan: number; text: string }) {
  return (
    <tr>
      <td colSpan={colSpan} className="px-4 py-10 text-center text-sm text-ink-500">
        {text}
      </td>
    </tr>
  )
}
