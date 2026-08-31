import { statusById } from '@/lib/mock-data'
import type { ToolStatus } from '@/lib/mock-data'
import { cn } from '@/lib/utils'

/** Цветная точка + подпись статуса (12px/600, цвет статуса) */
export function StatusDot({ status, className }: { status: ToolStatus; className?: string }) {
  const s = statusById(status)
  return (
    <span className={cn('inline-flex items-center gap-1.5 text-xs font-semibold', className)} style={{ color: s.color }}>
      <span className="w-1.5 h-1.5 rounded-full shrink-0" style={{ background: s.color }} />
      {s.name}
    </span>
  )
}

/** Статусный бейдж pill (uppercase, цветная пара фон/текст) */
export function StatusBadge({ status, className }: { status: ToolStatus; className?: string }) {
  const s = statusById(status)
  return (
    <span
      className={cn('inline-flex items-center rounded-full px-2.5 py-0.5 text-caption', className)}
      style={{ background: s.bg, color: s.color }}
    >
      {s.name}
    </span>
  )
}

/** QR-бейдж (teal) */
export function QrBadge({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 rounded-full bg-teal/20 px-2 py-0.5 text-[11px] font-semibold text-teal-dark',
        className
      )}
    >
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <rect x="3" y="3" width="7" height="7" rx="1" />
        <rect x="14" y="3" width="7" height="7" rx="1" />
        <rect x="3" y="14" width="7" height="7" rx="1" />
        <path d="M14 14h3v3h-3zM21 14v.01M14 21v.01M21 21v.01M18 18v.01" />
      </svg>
      QR
    </span>
  )
}

/** Бейдж «Материалы» (количественный учёт) */
export function MaterialBadge({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 rounded-full bg-brand-100/60 px-2 py-0.5 text-[11px] font-semibold text-ink-500',
        className
      )}
    >
      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <path d="M21 8l-9-5-9 5v8l9 5 9-5V8z" />
        <path d="M3 8l9 5 9-5M12 13v8" />
      </svg>
      Материалы
    </span>
  )
}
