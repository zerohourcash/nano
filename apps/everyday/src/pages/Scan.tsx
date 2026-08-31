import { useCallback, useEffect, useState } from 'react'
import { Link, useNavigate } from 'react-router'
import { ArrowLeft, Loader2, PackageCheck, Undo2, X } from 'lucide-react'
import { trpc } from '@/providers/trpc'
import { parseDueInput } from '@/lib/due-date'

import { useStore } from '@/lib/store'
import QrScanner from '@/components/QrScanner'
import { mapItemToCatalogTool } from '@/lib/catalog-item'
import type { CatalogTool } from '@/lib/catalog-item'

type BasketItem = CatalogTool & { rawId: number }

export default function Scan() {
  const navigate = useNavigate()
  const { currentUser } = useStore()
  const utils = trpc.useUtils()
  const [code, setCode] = useState<string | null>(null)
  const [toast, setToast] = useState<string | null>(null)
  const [basket, setBasket] = useState<BasketItem[]>([])

  const itemQ = trpc.items.byCode.useQuery(
    { code: code ?? '' },
    { enabled: Boolean(code), retry: false },
  )
  const takeMany = trpc.transfers.takeMany.useMutation({
    onSuccess: async (res) => {
      await utils.items.list.invalidate()
      await utils.meta.transferCounts.invalidate()
      await utils.items.byCode.invalidate()
      const taken = new Set(res.taken)
      setBasket((prev) => prev.filter((i) => !taken.has(i.rawId)))
      const skipped = res.failed.length
      setToast(
        skipped
          ? `Взято ${res.takenCount} шт., пропущено ${skipped}`
          : `Взято ${res.takenCount} шт.`,
      )
    },
    onError: (e) => setToast(e.message),
  })
  const ret = trpc.transfers.returnItem.useMutation({
    onSuccess: async () => {
      setToast('Возвращён на склад')
      await utils.items.list.invalidate()
      await utils.items.byCode.invalidate()
      await utils.meta.transferCounts.invalidate()
      setCode(null)
    },
    onError: (e) => setToast(e.message),
  })

  const onCode = useCallback((value: string) => {
    const trimmed = value.trim()
    try {
      const parsed = JSON.parse(trimmed) as { t?: string; token?: string; server?: string }
      if (parsed.t === 'join' && parsed.token) {
        const peer = parsed.server ? `&peer=${encodeURIComponent(parsed.server)}` : ''
        navigate(`/join?token=${encodeURIComponent(parsed.token)}${peer}`)
        return
      }
    } catch {
      /* not json */
    }
    const joinMatch = trimmed.match(/[?&]token=([^&]+)/)
    if (/\/join(\?|$)/.test(trimmed) && joinMatch) {
      const serverMatch = trimmed.match(/^https?:\/\/[^/?#]+(?::\d+)?/)
      const peer = serverMatch ? `&peer=${encodeURIComponent(serverMatch[0])}` : ''
      navigate(`/join?token=${encodeURIComponent(decodeURIComponent(joinMatch[1]))}${peer}`)
      return
    }
    setCode(trimmed)
  }, [navigate])

  useEffect(() => {
    if (!itemQ.data) return
    const catalog = mapItemToCatalogTool(itemQ.data)
    const itemId = itemQ.data.id
    const frame = requestAnimationFrame(() => {
      setBasket((prev) => {
        if (prev.some((i) => i.rawId === itemId)) return prev
        return [...prev, { ...catalog, rawId: itemId }]
      })
    })
    return () => cancelAnimationFrame(frame)
  }, [itemQ.data])

  useEffect(() => {
    if (!toast) return
    const t = setTimeout(() => setToast(null), 4000)
    return () => clearTimeout(t)
  }, [toast])

  const item = itemQ.data
  const last = item ? mapItemToCatalogTool(item) : null
  const isMine = item && currentUser && item.responsibleUserId === currentUser.id
  const takeIds = basket.map((i) => i.rawId)

  return (
    <div className="mx-auto max-w-lg space-y-4">
      <div className="flex items-center gap-3">
        <button
          onClick={() => navigate(-1)}
          className="flex h-10 w-10 items-center justify-center rounded-xl border border-brand-100 bg-white"
        >
          <ArrowLeft size={18} />
        </button>
        <div>
          <h1 className="text-xl font-bold text-ink-900">Сканировать QR</h1>
          <p className="text-[13px] text-ink-500">Сканируйте несколько бирок подряд, затем нажмите «Взять все»</p>
        </div>
      </div>

      <div className="rounded-card border border-brand-100/60 bg-surface p-4 shadow-card">
        <QrScanner onCode={onCode} />
      </div>

      {basket.length > 0 && (
        <div className="rounded-card border border-brand-100/60 bg-surface p-4 shadow-card space-y-3">
          <div className="flex items-center justify-between gap-2">
            <h2 className="text-[15px] font-semibold text-ink-900">
              К выдаче: <span className="font-mono-num text-accent">{basket.length}</span>
            </h2>
            <button
              onClick={() => setBasket([])}
              className="text-[13px] font-semibold text-brand-600 hover:bg-brand-50 rounded-lg px-2 py-1"
            >
              Очистить
            </button>
          </div>
          <ul className="space-y-2">
            {basket.map((b) => (
              <li key={b.rawId} className="flex items-center gap-3 rounded-xl border border-brand-100/70 px-3 py-2">
                <img src={b.photo} alt="" className="h-10 w-10 rounded-lg object-cover" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-semibold text-ink-900">{b.name}</span>
                  <span className="block font-mono-num text-xs text-ink-500">{b.vn}</span>
                </span>
                <button
                  onClick={() => setBasket((prev) => prev.filter((i) => i.rawId !== b.rawId))}
                  className="text-ink-300 hover:text-danger"
                  aria-label="Убрать"
                >
                  <X size={16} />
                </button>
              </li>
            ))}
          </ul>
          <button
            onClick={() => {
              const due = window.prompt('Срок возврата (ГГГГ-ММ-ДД ЧЧ:ММ). Пусто = без срока')
              if (due === null) return
              const parsed = parseDueInput(due)
              if (!parsed.ok) {
                setToast('Не понял дату. Формат: 2026-09-01 18:00')
                return
              }
              takeMany.mutate({ itemIds: takeIds, dueAt: parsed.iso })
            }}
            disabled={takeMany.isPending}
            className="inline-flex h-11 w-full items-center justify-center gap-2 rounded-xl bg-accent px-4 text-sm font-semibold text-white hover:bg-accent-hover disabled:opacity-60"
          >
            {takeMany.isPending ? <Loader2 size={16} className="animate-spin" /> : <PackageCheck size={16} />}
            Взять все ({basket.length})
          </button>
        </div>
      )}

      {code && (
        <div className="rounded-card border border-brand-100/60 bg-surface p-4 shadow-card space-y-3">
          <div className="text-[13px] text-ink-500">
            Последний код: <span className="font-mono-num font-semibold text-ink-900">{code}</span>
          </div>
          {itemQ.isLoading && (
            <div className="flex items-center gap-2 text-sm text-ink-500">
              <Loader2 size={16} className="animate-spin" /> Ищем инструмент…
            </div>
          )}
          {itemQ.isError && (
            <p className="text-sm font-semibold text-danger">Инструмент не найден. Проверьте номер.</p>
          )}
          {last && item && (
            <>
              <div className="text-sm text-ink-900">
                {last.name} · {last.vn}
                {last.assigneeName ? ` · у ${last.assigneeName}` : ' · на складе'}
              </div>
              {isMine && (
                <button
                  onClick={() => ret.mutate({ itemId: item.id })}
                  disabled={ret.isPending}
                  className="inline-flex h-11 items-center justify-center gap-2 rounded-xl border border-brand-100 bg-white px-4 text-sm font-semibold text-ink-900 hover:bg-brand-50 disabled:opacity-60"
                >
                  {ret.isPending ? <Loader2 size={16} className="animate-spin" /> : <Undo2 size={16} />}
                  Вернуть на склад
                </button>
              )}
              <Link
                to={`/tool/${item.id}`}
                className="inline-flex h-10 items-center justify-center rounded-xl text-sm font-semibold text-brand-600"
              >
                Открыть карточку
              </Link>
            </>
          )}
        </div>
      )}

      {toast && (
        <div className="fixed bottom-24 left-1/2 z-[60] -translate-x-1/2 rounded-full bg-ink-900 px-5 py-3 text-sm font-semibold text-white shadow-modal">
          {toast}
        </div>
      )}
    </div>
  )
}
