/**
 * Перевод предмета в «неисправен», «на ремонте» или «списан» требует причины
 * (ТЗ §8): она попадает в подписанный блок журнала, и сервер отклонит запрос
 * без неё. Здесь — общая для страниц проверка и запрос текста у пользователя.
 */
export const STATUSES_REQUIRING_REASON = ['in-repair', 'needs-check', 'written-off', 'broken']

export function statusNeedsReason(slug?: string | null): boolean {
  return Boolean(slug && STATUSES_REQUIRING_REASON.includes(slug))
}

/** Возвращает причину или null, если пользователь отменил ввод. */
export function askStatusReason(statusName: string): string | null {
  const answer = window.prompt(`Причина перевода в статус «${statusName}» (обязательно):`, '')
  if (answer === null) return null
  const trimmed = answer.trim()
  return trimmed.length >= 3 ? trimmed : null
}

/** Статусы, при которых предмет выведен из оборота: ни взять, ни передать. */
export const STATUSES_OUT_OF_CIRCULATION = ['in-repair', 'needs-check', 'written-off', 'broken']

export function itemCirculates(slug?: string | null): boolean {
  return !slug || !STATUSES_OUT_OF_CIRCULATION.includes(slug)
}
