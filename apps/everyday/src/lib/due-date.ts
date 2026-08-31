/** Разбор срока возврата, введённого руками. */
export function parseDueInput(raw: string): { ok: true; iso?: string } | { ok: false } {
  const value = raw.trim()
  if (!value) return { ok: true }
  const parsed = new Date(value.replace(' ', 'T'))
  if (Number.isNaN(parsed.getTime())) return { ok: false }
  return { ok: true, iso: parsed.toISOString() }
}

/**
 * Значение для <input type="datetime-local">: именно локальное время.
 * toISOString() здесь давал сдвиг на часовой пояс.
 */
export function toDateTimeLocal(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0')
  return (
    `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}` +
    `T${pad(date.getHours())}:${pad(date.getMinutes())}`
  )
}
