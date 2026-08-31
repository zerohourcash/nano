import { useEffect, useMemo, useState } from 'react'
import type { FormEvent } from 'react'
import { useNavigate, useSearchParams } from 'react-router'
import { Loader2, QrCode } from 'lucide-react'
import { trpc } from '@/providers/trpc'
import { isNativeApp } from '@/lib/app-mode'
import { toast } from 'sonner'

export default function Join() {
  const [params] = useSearchParams()
  const token = params.get('token') ?? ''
  const peer = (params.get('peer') ?? params.get('server') ?? '').replace(/\/$/, '')
  const navigate = useNavigate()
  const utils = trpc.useUtils()
  const addPeer = trpc.sync.addPeer.useMutation()
  const pullNow = trpc.sync.pullNow.useMutation()
  const [peerReady, setPeerReady] = useState(!peer)
  const [peerError, setPeerError] = useState<string | null>(null)

  useEffect(() => {
    if (!peer) return
    let cancelled = false
    ;(async () => {
      try {
        setPeerError(null)
        if (isNativeApp()) {
          await addPeer.mutateAsync({ url: peer, name: 'invite' })
          await pullNow.mutateAsync({ url: peer })
        }
        if (!cancelled) setPeerReady(true)
      } catch (e) {
        if (!cancelled) {
          setPeerError(e instanceof Error ? e.message : 'Не удалось связаться с узлом приглашения')
          setPeerReady(true)
        }
      }
    })()
    return () => {
      cancelled = true
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [peer])

  const info = trpc.auth.inviteInfo.useQuery(
    { token },
    { enabled: token.length > 8 && peerReady, retry: false },
  )
  const me = trpc.auth.me.useQuery(undefined, { retry: false })
  const loggedIn = Boolean(me.data)
  const join = trpc.auth.join.useMutation()
  const joinReg = trpc.auth.joinRegister.useMutation()

  const [name, setName] = useState('')
  const [phone, setPhone] = useState('')
  const [password, setPassword] = useState('')
  const [busy, setBusy] = useState(false)

  const wsName = useMemo(() => info.data?.workspace?.name ?? 'группу', [info.data])

  const after = async () => {
    await utils.invalidate()
    toast.success(`Вы в «${wsName}»`)
    navigate('/')
  }

  const onJoinExisting = async () => {
    try {
      setBusy(true)
      await join.mutateAsync({ token })
      await after()
    } catch (e) {
      toast.error(e instanceof Error ? e.message : 'Не удалось вступить')
    } finally {
      setBusy(false)
    }
  }

  const onRegisterJoin = async (e: FormEvent) => {
    e.preventDefault()
    if (!name.trim() || phone.replace(/\D/g, '').length < 11 || password.length < 10) {
      toast.error('Имя, полный телефон и пароль от 12 символов')
      return
    }
    try {
      setBusy(true)
      await joinReg.mutateAsync({ token, fullName: name.trim(), phone, password })
      await after()
    } catch (err) {
      toast.error(err instanceof Error ? err.message : 'Не удалось присоединиться')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="min-h-[100dvh] bg-app flex items-center justify-center px-4">
      <div className="w-full max-w-md rounded-card border border-brand-100/60 bg-surface p-6 shadow-card space-y-4">
        <div className="flex items-center gap-3">
          <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-brand-50 text-brand-600">
            <QrCode size={22} />
          </div>
          <div>
            <h1 className="text-xl font-bold text-ink-900">Приглашение в группу</h1>
            <p className="text-sm text-ink-500">{info.isError ? 'Код недействителен' : `«${wsName}»`}</p>
          </div>
        </div>

        {!peerReady && <p className="text-sm text-ink-500">Подтягиваем журнал с узла по Wi‑Fi…</p>}
        {peerError && (
          <p className="text-sm font-semibold text-danger">
            {peerError}. Подключитесь к той же Wi‑Fi, что и организатор, и откройте QR снова.
          </p>
        )}
        {info.isLoading && <p className="text-sm text-ink-500">Проверяем приглашение…</p>}
        {info.isError && (
          <p className="text-sm font-semibold text-danger">QR-приглашение не найдено или уже использовано.</p>
        )}

        {info.data && loggedIn && (
          <button
            onClick={() => void onJoinExisting()}
            disabled={busy}
            className="flex h-12 w-full items-center justify-center gap-2 rounded-xl bg-accent text-sm font-semibold text-white hover:bg-accent-hover disabled:opacity-60"
          >
            {busy ? <Loader2 className="animate-spin" size={18} /> : null}
            Вступить
          </button>
        )}

        {info.data && !loggedIn && (
          <form className="space-y-3" onSubmit={(e) => void onRegisterJoin(e)}>
            <p className="text-sm text-ink-500">Создайте аккаунт или войдите тем же телефоном — и сразу попадёте в группу.</p>
            <input className="h-11 w-full rounded-xl border border-brand-100 px-3" placeholder="Имя и фамилия" value={name} onChange={(e) => setName(e.target.value)} />
            <input className="h-11 w-full rounded-xl border border-brand-100 px-3" placeholder="+7 921 000-00-00" value={phone} onChange={(e) => setPhone(e.target.value)} />
            <input className="h-11 w-full rounded-xl border border-brand-100 px-3" type="password" placeholder="Пароль от 12 символов" value={password} onChange={(e) => setPassword(e.target.value)} />
            <button disabled={busy} className="flex h-12 w-full items-center justify-center gap-2 rounded-xl bg-accent text-sm font-semibold text-white hover:bg-accent-hover disabled:opacity-60">
              {busy ? <Loader2 className="animate-spin" size={18} /> : null}
              Зарегистрироваться и вступить
            </button>
            <button type="button" className="w-full text-sm font-semibold text-brand-600" onClick={() => navigate(`/login?join=${token}`)}>
              У меня уже есть аккаунт
            </button>
          </form>
        )}
      </div>
    </div>
  )
}
