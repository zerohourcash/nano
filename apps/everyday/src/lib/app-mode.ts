export const DEFAULT_RELAY = ''

export function isNativeApp(): boolean {
  if (typeof window === 'undefined') return false
  try {
    if (/MeshKeeperAndroid/i.test(navigator.userAgent)) return true
    const q = new URLSearchParams(window.location.search)
    return q.get('app') === '1'
  } catch {
    return false
  }
}

export function nativeLanOrigin(): string {
  if (typeof window === 'undefined') return ''
  try {
    const w = window as unknown as {
      __MK_LAN__?: string
      MeshKeeperNative?: { lanOrigin?: () => string }
    }
    const fromJs = w.MeshKeeperNative?.lanOrigin?.()
    if (fromJs && /^https?:\/\//i.test(fromJs)) return fromJs.replace(/\/$/, '')
    if (typeof w.__MK_LAN__ === 'string' && w.__MK_LAN__) return w.__MK_LAN__.replace(/\/$/, '')
  } catch {
    /* ignore */
  }
  return ''
}

export function trpcUrl(): string {
  return '/api/trpc'
}

function cleanOrigin(raw?: string | null): string {
  if (!raw) return ''
  const v = raw.trim().replace(/\/$/, '')
  return /^https?:\/\//i.test(v) ? v : ''
}

function isLoopback(origin: string): boolean {
  return /127\.0\.0\.1|localhost/i.test(origin)
}

export function publicJoinOrigin(syncUrl?: string | null): string {
  const lan = nativeLanOrigin()
  if (lan && !isLoopback(lan)) return lan
  const sync = cleanOrigin(syncUrl)
  if (sync && !isLoopback(sync)) return sync
  if (typeof window !== 'undefined') {
    const origin = window.location.origin.replace(/\/$/, '')
    if (origin && !isLoopback(origin)) return origin
  }
  if (lan) return lan
  return DEFAULT_RELAY
}

export function joinInviteUrl(
  token: string,
  opts?: { syncUrl?: string | null; lanUrl?: string | null },
): string {
  const origin =
    publicJoinOrigin(opts?.syncUrl) ||
    cleanOrigin(opts?.lanUrl) ||
    (typeof window !== 'undefined' ? window.location.origin : DEFAULT_RELAY)
  return `${origin.replace(/\/$/, '')}/join?token=${encodeURIComponent(token)}`
}
