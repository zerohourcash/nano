export type InviteRole = 'member' | 'admin' | 'viewer'

export const INVITE_ROLES: Array<{ value: InviteRole; label: string; hint: string }> = [
  { value: 'member', label: 'Участник', hint: 'берёт и возвращает оборудование' },
  { value: 'admin', label: 'Администратор', hint: 'ведёт каталог, заявки и инвентаризацию' },
  { value: 'viewer', label: 'Наблюдатель', hint: 'только смотрит каталог' },
]

export const INVITE_TTL_HOURS = 168

type InviteLike = {
  token: string
  usable?: boolean
  expired?: boolean
  revoked?: boolean
  usedCount?: number
  maxUses?: number
}

/**
 * Первое приглашение, которым ещё можно воспользоваться. Показывать
 * просроченный или исчерпанный QR нельзя: он отвалится при сканировании.
 */
export function firstUsableInvite<T extends InviteLike>(list?: T[] | null): T | undefined {
  return list?.find((invite) => {
    if (typeof invite.usable === 'boolean') return invite.usable
    if (invite.revoked || invite.expired) return false
    if (typeof invite.usedCount === 'number' && typeof invite.maxUses === 'number') {
      return invite.usedCount < invite.maxUses
    }
    return true
  })
}

export function inviteExpiryLabel(expiresAt?: string | null): string | null {
  if (!expiresAt) return null
  const date = new Date(expiresAt)
  if (Number.isNaN(date.getTime())) return null
  return date.toLocaleString('ru-RU', { dateStyle: 'short', timeStyle: 'short' })
}
