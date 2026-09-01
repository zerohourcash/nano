import { useMemo, useState } from 'react'
import { motion } from 'framer-motion'
import { ArrowDownLeft, ArrowUpRight, Coins, Loader2, ReceiptText, ShieldCheck } from 'lucide-react'
import { toast } from 'sonner'
import { trpc } from '@/providers/trpc'
import { useStore } from '@/lib/store'
import { cn } from '@/lib/utils'

type Mode = 'transfer' | 'sale' | 'mint'

export default function BitWallet() {
  const { workspace, currentUser } = useStore()
  const workspaceId = workspace?.id
  const meQ = trpc.auth.me.useQuery()
  const rights = (meQ.data?.roleRights ?? {}) as Record<string, unknown>
  const canMint = rights.manageAccounting === true
  const canAudit = rights.viewAccounting === true
  const [mode, setMode] = useState<Mode>('transfer')
  const [recipientId, setRecipientId] = useState('')
  const [itemId, setItemId] = useState('')
  const [amount, setAmount] = useState('')
  const [memo, setMemo] = useState('')
  const utils = trpc.useUtils()

  const balanceQ = trpc.bit.balance.useQuery({ workspaceId }, { enabled: !!workspaceId })
  const recipientsQ = trpc.bit.recipients.useQuery({ workspaceId }, { enabled: !!workspaceId })
  const mineQ = trpc.bit.myTransactions.useQuery({ workspaceId }, { enabled: !!workspaceId && !canAudit })
  const allQ = trpc.bit.transactions.useQuery({ workspaceId }, { enabled: !!workspaceId && canAudit })
  const itemsQ = trpc.items.list.useQuery(
    { workspaceId, page: 1, limit: 100, sort: 'title_asc' },
    { enabled: !!workspaceId && mode === 'sale' }
  )
  const transactions = canAudit ? allQ.data ?? [] : mineQ.data ?? []
  const recipients = useMemo(() => recipientsQ.data ?? [], [recipientsQ.data])
  const recipientNames = useMemo(
    () => new Map(recipients.map((user) => [user.id, user.fullName])),
    [recipients]
  )

  const refresh = async () => {
    await Promise.all([
      utils.bit.balance.invalidate(),
      utils.bit.myTransactions.invalidate(),
      utils.bit.transactions.invalidate(),
      utils.sync.audit.invalidate(),
    ])
    setAmount('')
    setMemo('')
  }
  const mutationOptions = {
    onSuccess: async () => {
      await refresh()
      toast.success('Bit-транзакция подписана и записана в летопись')
    },
    onError: (error: { message: string }) => toast.error(error.message),
  }
  const transfer = trpc.bit.transfer.useMutation(mutationOptions)
  const mint = trpc.bit.mint.useMutation(mutationOptions)
  const sale = trpc.bit.sale.useMutation(mutationOptions)
  const pending = transfer.isPending || mint.isPending || sale.isPending

  const submit = (event: React.FormEvent) => {
    event.preventDefault()
    if (!workspaceId) return
    const units = Number(amount)
    const target = Number(recipientId)
    if (!Number.isSafeInteger(units) || units <= 0 || !target) {
      toast.error('Укажите получателя и целое положительное количество Bit')
      return
    }
    if (mode === 'mint') {
      mint.mutate({ workspaceId, recipientUserId: target, amount: units, memo: memo.trim() || undefined })
    } else if (mode === 'sale') {
      const item = Number(itemId)
      if (!item) {
        toast.error('Выберите товар или ТМЦ')
        return
      }
      sale.mutate({ itemId: item, sellerUserId: target, amount: units, memo: memo.trim() || undefined })
    } else {
      transfer.mutate({ workspaceId, recipientUserId: target, amount: units, memo: memo.trim() || undefined })
    }
  }

  return (
    <div className="space-y-5 pb-20 lg:pb-0">
      <div>
        <h1 className="text-2xl font-bold text-ink-900">Кошелёк Bit</h1>
        <p className="mt-1 text-sm text-ink-500">Локальные расчёты с двойной бухгалтерской записью</p>
      </div>

      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(320px,420px)]">
        <motion.section initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} className="rounded-card border border-brand-100/60 bg-gradient-to-br from-brand-700 to-brand-500 p-6 text-white shadow-card">
          <div className="flex items-center gap-2 text-sm text-white/75"><Coins size={18} /> Доступный баланс</div>
          <div data-testid="bit-balance" className="mt-4 font-mono text-4xl font-semibold tracking-tight">
            {balanceQ.isLoading ? '…' : `${balanceQ.data?.balance ?? 0} Bit`}
          </div>
          <div className="mt-5 flex items-center gap-2 text-xs text-white/75"><ShieldCheck size={15} /> Проводки подписываются устройством и сходятся после восстановления mesh</div>
        </motion.section>

        <form onSubmit={submit} className="rounded-card border border-brand-100/60 bg-surface p-5 shadow-card">
          <div className="flex rounded-xl bg-brand-50 p-1 text-sm font-semibold">
            {(['transfer', 'sale', ...(canMint ? ['mint' as const] : [])] as Mode[]).map((value) => (
              <button key={value} type="button" onClick={() => setMode(value)} className={cn('flex-1 rounded-lg px-2 py-2 transition', mode === value ? 'bg-white text-brand-700 shadow-sm' : 'text-ink-500')}>
                {value === 'transfer' ? 'Перевод' : value === 'sale' ? 'Покупка' : 'Эмиссия'}
              </button>
            ))}
          </div>
          <label className="mt-4 block text-sm font-semibold text-ink-900">
            {mode === 'sale' ? 'Продавец' : 'Получатель'}
            <select aria-label={mode === 'sale' ? 'Продавец' : 'Получатель Bit'} value={recipientId} onChange={(event) => { setRecipientId(event.target.value); if (mode === 'sale') setItemId('') }} className="mt-1.5 h-11 w-full rounded-xl border border-brand-100 bg-white px-3 font-normal">
              <option value="">Выберите участника</option>
              {recipients.filter((user) => mode === 'mint' || user.id !== currentUser?.id).map((user) => <option key={user.id} value={user.id}>{user.fullName}{user.position ? ` · ${user.position}` : ''}</option>)}
            </select>
          </label>
          {mode === 'sale' && (
            <label className="mt-3 block text-sm font-semibold text-ink-900">Товар / ТМЦ
              <select aria-label="Товар или ТМЦ" value={itemId} onChange={(event) => setItemId(event.target.value)} className="mt-1.5 h-11 w-full rounded-xl border border-brand-100 bg-white px-3 font-normal">
                <option value="">Выберите позицию</option>
                {(itemsQ.data?.rows ?? []).filter((item) => item.responsibleUserId === Number(recipientId)).map((item) => <option key={item.id} value={item.id}>{item.internalId} · {item.title}</option>)}
              </select>
            </label>
          )}
          <label className="mt-3 block text-sm font-semibold text-ink-900">Сумма
            <input aria-label="Сумма Bit" value={amount} onChange={(event) => setAmount(event.target.value.replace(/\D/g, ''))} inputMode="numeric" className="mt-1.5 h-11 w-full rounded-xl border border-brand-100 px-3 font-mono" placeholder="100" />
          </label>
          <label className="mt-3 block text-sm font-semibold text-ink-900">Назначение
            <input aria-label="Назначение платежа" value={memo} onChange={(event) => setMemo(event.target.value)} maxLength={500} className="mt-1.5 h-11 w-full rounded-xl border border-brand-100 px-3 font-normal" placeholder="Оплата работы, покупка…" />
          </label>
          <button disabled={pending} className="mt-4 inline-flex h-11 w-full items-center justify-center gap-2 rounded-xl bg-accent font-semibold text-white hover:bg-accent-hover disabled:opacity-60">
            {pending && <Loader2 size={17} className="animate-spin" />}
            Подписать транзакцию
          </button>
        </form>
      </div>

      <section className="rounded-card border border-brand-100/60 bg-surface shadow-card">
        <div className="flex items-center justify-between border-b border-brand-100/60 px-5 py-4">
          <h2 className="flex items-center gap-2 font-semibold text-ink-900"><ReceiptText size={18} /> {canAudit ? 'Бухгалтерская летопись' : 'Мои операции'}</h2>
          <span className="text-xs text-ink-300">{transactions.length} операций</span>
        </div>
        {transactions.length === 0 ? <p className="p-8 text-center text-sm text-ink-500">Операций Bit пока нет</p> : (
          <div className="divide-y divide-brand-100/60">
            {transactions.map((transaction) => {
              const incoming = transaction.recipientUserId === currentUser?.id
              const outgoing = transaction.senderUserId === currentUser?.id
              const counterparty = incoming ? transaction.senderUserId : transaction.recipientUserId
              return <article key={transaction.guid} data-testid="bit-transaction" className="flex items-center gap-3 px-5 py-4">
                <span className={cn('flex h-10 w-10 items-center justify-center rounded-full', incoming ? 'bg-success-bg text-success' : 'bg-brand-50 text-brand-600')}>
                  {incoming ? <ArrowDownLeft size={19} /> : <ArrowUpRight size={19} />}
                </span>
                <div className="min-w-0 flex-1"><div className="truncate font-semibold text-ink-900">{transaction.memo || (transaction.kind === 'mint' ? 'Эмиссия Bit' : transaction.kind === 'sale' ? 'Покупка' : 'Перевод')}</div><div className="text-xs text-ink-500">{counterparty ? recipientNames.get(counterparty) ?? `Участник #${counterparty}` : 'Системный счёт'} · {new Date(transaction.createdAt).toLocaleString('ru-RU')}</div></div>
                <div className={cn('font-mono font-semibold', incoming ? 'text-success' : outgoing ? 'text-danger' : 'text-ink-900')}>{incoming ? '+' : outgoing ? '−' : ''}{transaction.amount} Bit<div className="text-right text-[10px] font-normal text-ink-300">{transaction.status === 'posted' ? 'подтверждено' : 'конфликт'}</div></div>
              </article>
            })}
          </div>
        )}
      </section>
    </div>
  )
}
