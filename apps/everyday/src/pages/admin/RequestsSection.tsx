import { useState } from 'react'
import { AlertTriangle, Check, Loader2, X } from 'lucide-react'
import { format } from 'date-fns'
import { ru } from 'date-fns/locale'
import { trpc } from '@/providers/trpc'
import { useStore } from '@/lib/store'
import { btnPrimaryCls, btnSecondaryCls, cardCls, useToast } from './ui'

type ChangeRow = {
  field: string
  label: string
  before: string | null
  after: string | null
}

/** Сравнение «было / предлагается» — администратор решает по нему, а не по JSON. */
function ChangeDiff({ changes }: { changes: ChangeRow[] }) {
  if (!changes.length) {
    return (
      <p className="text-[13px] text-ink-300">
        Заявка не меняет значений — возможно, карточку уже поправили.
      </p>
    )
  }
  return (
    <div className="overflow-x-auto rounded-xl border border-brand-100/70">
      <table className="w-full text-[13px]">
        <thead>
          <tr className="bg-brand-50 text-ink-500">
            <th className="px-3 py-1.5 text-left font-semibold">Поле</th>
            <th className="px-3 py-1.5 text-left font-semibold">Было</th>
            <th className="px-3 py-1.5 text-left font-semibold">Предлагается</th>
          </tr>
        </thead>
        <tbody>
          {changes.map((row) => (
            <tr key={row.field} className="border-t border-brand-100/60">
              <td className="px-3 py-1.5 text-ink-500">{row.label}</td>
              <td className="px-3 py-1.5 text-ink-500 line-through decoration-ink-300">
                {row.before ?? '—'}
              </td>
              <td className="px-3 py-1.5 font-semibold text-ink-900">{row.after ?? '—'}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

export default function RequestsSection() {
  const toast = useToast()
  const { workspace } = useStore()
  const utils = trpc.useUtils()
  const wsId = workspace?.id
  const faultsQ = trpc.items.faults.useQuery({ workspaceId: wsId })
  const changesQ = trpc.items.changeRequests.useQuery({ workspaceId: wsId })
  const [reason, setReason] = useState('')

  const resolve = trpc.items.resolveFault.useMutation({
    onSuccess: () => {
      utils.items.faults.invalidate()
      utils.items.list.invalidate()
      toast('Неисправность обработана')
    },
    onError: (e) => toast(e.message, 'error'),
  })
  const decide = trpc.items.decideChange.useMutation({
    onSuccess: () => {
      utils.items.changeRequests.invalidate()
      utils.items.list.invalidate()
      toast('Заявка рассмотрена')
    },
    onError: (e) => toast(e.message, 'error'),
  })

  const faults = faultsQ.data ?? []
  const changes = changesQ.data ?? []

  return (
    <div className="space-y-6">
      <section className={cardCls}>
        <h3 className="text-[17px] font-semibold text-ink-900 mb-3">Неисправности</h3>
        {faults.length === 0 && <p className="text-sm text-ink-500">Открытых сообщений нет.</p>}
        <div className="space-y-2">
          {faults.map((f) => (
            <div key={f.id} className="rounded-xl border border-brand-100/70 p-3 space-y-2">
              <div className="flex items-start gap-2">
                <AlertTriangle size={16} className="text-[#A87C0F] mt-0.5 shrink-0" />
                <div className="min-w-0 flex-1">
                  <div className="text-sm font-semibold text-ink-900">
                    #{f.id} · {f.severity} · {f.status}
                  </div>
                  <p className="text-sm text-ink-500">{f.description}</p>
                  <p className="text-[12px] text-ink-300">
                    {f.author?.fullName ?? 'Участник'} · {format(new Date(f.createdAt), 'dd.MM.yyyy HH:mm', { locale: ru })}
                  </p>
                </div>
              </div>
              {f.status === 'open' && (
                <div className="flex flex-wrap gap-2">
                  <button
                    className={btnPrimaryCls}
                    disabled={resolve.isPending}
                    onClick={() => resolve.mutate({ id: f.id, status: 'repair', comment: 'Переведён в ремонт' })}
                  >
                    {resolve.isPending ? <Loader2 size={14} className="animate-spin" /> : null}
                    В ремонт
                  </button>
                  <button
                    className={btnSecondaryCls}
                    disabled={resolve.isPending}
                    onClick={() => resolve.mutate({ id: f.id, status: 'resolved', comment: 'Проверено, исправен' })}
                  >
                    <Check size={14} />
                    Исправен
                  </button>
                </div>
              )}
            </div>
          ))}
        </div>
      </section>

      <section className={cardCls}>
        <h3 className="text-[17px] font-semibold text-ink-900 mb-3">Заявки на правку карточек</h3>
        {changes.length === 0 && <p className="text-sm text-ink-500">Заявок нет.</p>}
        <div className="space-y-2">
          {changes.map((c) => (
            <div key={c.id} className="rounded-xl border border-brand-100/70 p-3 space-y-2">
              <div className="text-sm font-semibold text-ink-900">
                {c.item?.internalId ?? `#${c.itemId}`} {c.item?.title ?? ''} · {c.status}
              </div>
              <p className="text-sm text-ink-500">{c.comment ?? 'Без комментария'}</p>
              <ChangeDiff changes={c.changes} />
              {c.status === 'pending' && (
                <div className="space-y-2">
                  <input
                    value={reason}
                    onChange={(e) => setReason(e.target.value)}
                    placeholder="Причина отклонения (если отклоняете)"
                    className="w-full h-10 rounded-xl border border-brand-100 px-3 text-sm"
                  />
                  <div className="flex gap-2">
                    <button
                      className={btnPrimaryCls}
                      disabled={decide.isPending}
                      onClick={() => decide.mutate({ id: c.id, accept: true })}
                    >
                      <Check size={14} /> Принять
                    </button>
                    <button
                      className={btnSecondaryCls}
                      disabled={decide.isPending}
                      onClick={() => decide.mutate({ id: c.id, accept: false, reason: reason || 'Отклонено' })}
                    >
                      <X size={14} /> Отклонить
                    </button>
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>
      </section>
    </div>
  )
}
