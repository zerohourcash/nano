import { useCallback, useEffect, useMemo, useState } from 'react'
import { Link } from 'react-router'
import { Copy, Loader2, QrCode, RefreshCw } from 'lucide-react'
import { trpc } from '@/providers/trpc'
import { useStore } from '@/lib/store'
import { joinInviteUrl } from '@/lib/app-mode'
import InviteQrBlock from '@/components/InviteQrBlock'
import { INVITE_ROLES, INVITE_TTL_HOURS, firstUsableInvite, inviteExpiryLabel } from '@/lib/invite'
import type { InviteRole } from '@/lib/invite'

export default function Invite() {
  const { workspace, workspaces } = useStore()
  const utils = trpc.useUtils()
  const [url, setUrl] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  const [role, setRole] = useState<InviteRole>('member')
  const [expiresAt, setExpiresAt] = useState<string | null>(null)
  const wsQ = trpc.admin.workspaces.list.useQuery()
  const syncQ = trpc.sync.status.useQuery()
  const current = useMemo(() => {
    const list = wsQ.data?.length ? wsQ.data : workspaces
    return list.find((w) => w.id === workspace?.id) ?? list[0] ?? workspace ?? null
  }, [wsQ.data, workspaces, workspace])
  const wsId = current?.id
  const invitesQ = trpc.admin.workspaces.invites.useQuery(
    { workspaceId: wsId },
    { enabled: Boolean(wsId), retry: false },
  )

  const makeUrl = useCallback(
    (token: string) =>
      joinInviteUrl(token, {
        syncUrl: (current as { syncUrl?: string | null } | null)?.syncUrl,
        lanUrl: syncQ.data?.url,
      }),
    [current, syncQ.data?.url],
  )

  const createInvite = trpc.admin.workspaces.createInvite.useMutation({
    onSuccess: (res) => {
      if (res?.token) setUrl(makeUrl(res.token))
      setExpiresAt(res?.expiresAt ?? null)
    },
  })

  const issue = useCallback(
    (nextRole: InviteRole) => {
      if (!wsId) return
      setUrl(null)
      setExpiresAt(null)
      createInvite.mutate({
        workspaceId: wsId,
        role: nextRole,
        maxUses: 50,
        expiresInHours: INVITE_TTL_HOURS,
      })
    },
    [wsId, createInvite],
  )

  useEffect(() => {
    if (!wsId || url) return
    const existing = firstUsableInvite(invitesQ.data)
    if (existing) {
      const frame = requestAnimationFrame(() => {
        setUrl(makeUrl(existing.token))
        setExpiresAt(existing.expiresAt ?? null)
      })
      return () => cancelAnimationFrame(frame)
    }
    if (invitesQ.isLoading || createInvite.isPending || createInvite.isSuccess || createInvite.isError) return
    if (invitesQ.isFetched)
      createInvite.mutate({ workspaceId: wsId, role, maxUses: 50, expiresInHours: INVITE_TTL_HOURS })
  }, [wsId, url, role, invitesQ.data, invitesQ.isLoading, invitesQ.isFetched, createInvite, makeUrl])

  const copy = async () => {
    if (!url) return
    await navigator.clipboard.writeText(url)
    setCopied(true)
    window.setTimeout(() => setCopied(false), 2000)
  }

  const busy = !url && (wsQ.isLoading || invitesQ.isLoading || createInvite.isPending)

  return (
    <div className="mx-auto max-w-2xl space-y-5">
      <div>
        <h1 className="text-2xl lg:text-[28px] leading-9 font-bold tracking-[-0.01em] text-ink-900">
          Пригласить по QR
        </h1>
        <p className="mt-1 text-sm text-ink-500">
          Покажите этот код с экрана. Второй человек сканирует его в приложении Everyday
          и сразу попадает в «{current?.name ?? workspace?.name ?? 'группу'}».
        </p>
      </div>

      <section className="bg-surface rounded-card border border-brand-100/60 shadow-card p-6">
        {busy && (
          <div className="flex items-center justify-center gap-2 py-16 text-sm text-ink-500">
            <Loader2 className="animate-spin" size={18} /> Готовим QR-приглашение…
          </div>
        )}
        {!busy && !wsId && (
          <p className="text-sm font-semibold text-danger">
            Сначала создайте организацию — без группы QR выпустить нельзя.
          </p>
        )}
        {createInvite.isError && !url && (
          <p className="text-sm font-semibold text-danger">
            {createInvite.error.message}
            <button className="ml-2 text-brand-600" onClick={() => issue(role)}>
              Повторить
            </button>
          </p>
        )}
        {url && (
          <div className="flex flex-col items-center gap-5 sm:flex-row sm:items-start">
            <InviteQrBlock value={url} size={240} />
            <div className="min-w-0 flex-1 space-y-3">
              <div className="flex items-center gap-2 text-brand-600">
                <QrCode size={18} />
                <span className="text-sm font-semibold">Код группы</span>
              </div>
              <p className="break-all font-mono-num text-[13px] text-ink-500">{url}</p>
              <div className="flex flex-wrap items-center gap-2">
                {INVITE_ROLES.map((r) => (
                  <button
                    key={r.value}
                    type="button"
                    title={r.hint}
                    onClick={() => {
                      setRole(r.value)
                      issue(r.value)
                    }}
                    className={
                      'h-9 rounded-xl border px-3 text-[13px] font-semibold ' +
                      (role === r.value
                        ? 'border-accent bg-accent/10 text-accent'
                        : 'border-brand-100 bg-white text-ink-500 hover:bg-brand-50')
                    }
                  >
                    {r.label}
                  </button>
                ))}
              </div>
              {inviteExpiryLabel(expiresAt) && (
                <p className="text-[13px] leading-[18px] text-ink-500">
                  Код действует до {inviteExpiryLabel(expiresAt)}. После этого выпустите новый.
                </p>
              )}
              <div className="flex flex-wrap gap-2">
                <button
                  type="button"
                  onClick={() => void copy()}
                  className="inline-flex h-10 items-center gap-2 rounded-xl bg-accent px-5 text-sm font-semibold text-white hover:bg-accent-hover"
                >
                  <Copy size={16} />
                  {copied ? 'Скопировано' : 'Скопировать ссылку'}
                </button>
                <button
                  type="button"
                  onClick={() => {
                    issue(role)
                    void utils.admin.workspaces.invites.invalidate()
                  }}
                  className="inline-flex h-10 items-center gap-2 rounded-xl border border-brand-100 bg-white px-4 text-sm font-semibold text-ink-900 hover:bg-brand-50"
                >
                  <RefreshCw size={16} />
                  Новый код
                </button>
              </div>
              <p className="text-[13px] leading-[18px] text-ink-500">
                Наведите камеру приложения на квадрат. Интернет не обязателен — достаточно одной Wi‑Fi.
              </p>
              <Link to="/admin" className="inline-block text-[13px] font-semibold text-brand-600 hover:underline">
                Настройки группы
              </Link>
            </div>
          </div>
        )}
      </section>
    </div>
  )
}
