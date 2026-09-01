import { useMemo, useState } from 'react'
import { Ban, Check, Clipboard, KeyRound, Plus, Send, ShieldCheck } from 'lucide-react'
import { trpc } from '@/providers/trpc'
import { useStore } from '@/lib/store'
import InviteQrBlock from '@/components/InviteQrBlock'
import { decodeCard, encodeCard } from '@/lib/interorg-card'
import { SectionHeader, btnPrimaryCls, btnSecondaryCls, cardCls, inputCls, useToast } from './ui'

export default function InterorgSection() {
  const { workspace } = useStore()
  const workspaceId = workspace?.id ?? 0
  const toast = useToast()
  const identityQ = trpc.interorg.identity.useQuery({ workspaceId }, { enabled: workspaceId > 0 })
  const contactsQ = trpc.interorg.contacts.useQuery({ workspaceId }, { enabled: workspaceId > 0 })
  const inboxQ = trpc.interorg.inbox.useQuery(
    { workspaceId },
    { enabled: workspaceId > 0, refetchInterval: 5000 },
  )
  const outboxQ = trpc.interorg.outbox.useQuery(
    { workspaceId },
    { enabled: workspaceId > 0, refetchInterval: 5000 },
  )
  const [cardText, setCardText] = useState('')
  const [contactName, setContactName] = useState('')
  const [selected, setSelected] = useState('')
  const [kind, setKind] = useState('message.notice')
  const [message, setMessage] = useState('')

  const ensure = trpc.interorg.ensureIdentity.useMutation({
    onSuccess: () => { void identityQ.refetch(); toast('Адрес организации создан') },
    onError: (error) => toast(error.message, 'error'),
  })
  const trust = trpc.interorg.trustContact.useMutation({
    onSuccess: (contact) => {
      void contactsQ.refetch()
      setSelected(contact.guid)
      setCardText('')
      setContactName('')
      toast('Контрагент добавлен в доверенный каталог')
    },
    onError: (error) => toast(error.message, 'error'),
  })
  const revoke = trpc.interorg.revokeContact.useMutation({
    onSuccess: () => {
      void contactsQ.refetch()
      setSelected('')
      toast('Доверие к ключам контрагента отозвано')
    },
    onError: (error) => toast(error.message, 'error'),
  })
  const send = trpc.interorg.send.useMutation({
    onSuccess: () => { setMessage(''); void outboxQ.refetch(); toast('Транзакция подписана и поставлена в mesh-очередь') },
    onError: (error) => toast(error.message, 'error'),
  })
  const accept = trpc.interorg.accept.useMutation({
    onSuccess: () => { void inboxQ.refetch(); toast('Принятие записано в летопись') },
    onError: (error) => toast(error.message, 'error'),
  })

  const ownCard = useMemo(() => {
    const identity = identityQ.data
    if (!identity || !workspace?.guid) return ''
    return encodeCard({
      v: 1,
      type: 'everyday-org',
      workspaceGuid: workspace.guid,
      name: workspace.name,
      encryptionKey: identity.publicKey,
      signingKey: identity.signingKey,
    })
  }, [identityQ.data, workspace])

  const addContact = () => {
    try {
      const card = decodeCard(cardText)
      trust.mutate({
        workspaceId,
        name: contactName.trim() || card.name,
        remoteWorkspaceGuid: card.workspaceGuid,
        encryptionKey: card.encryptionKey,
        signingKey: card.signingKey,
      })
    } catch (error) {
      toast(error instanceof Error ? error.message : 'Некорректная визитка', 'error')
    }
  }

  if (!workspaceId) return <div className={cardCls + ' p-6 text-sm text-ink-500'}>Выберите организацию.</div>

  return (
    <div className="space-y-5" data-testid="interorg-network">
      <div>
        <SectionHeader title="Сеть организаций" />
        <p className="mt-1 text-sm text-ink-500">Изолированные летописи, адресные зашифрованные транзакции через любые relay-узлы</p>
      </div>

      <section className={`${cardCls} p-5 sm:p-6`}>
        <div className="flex items-start justify-between gap-4">
          <div>
            <h3 className="font-semibold text-ink-900">Криптографическая визитка</h3>
            <p className="mt-1 text-sm text-ink-500">Передайте её контрагенту по QR и подтвердите отпечаток другим каналом.</p>
          </div>
          <KeyRound className="shrink-0 text-brand-600" size={22} />
        </div>
        {!identityQ.data ? (
          <button className={`${btnPrimaryCls} mt-5`} onClick={() => ensure.mutate({ workspaceId })} disabled={ensure.isPending}>
            <Plus size={17} /> Создать адрес
          </button>
        ) : (
          <div className="mt-5 grid gap-5 md:grid-cols-[auto_1fr]">
            <InviteQrBlock value={ownCard} size={164} />
            <div className="min-w-0 space-y-3">
              <div className="rounded-xl bg-brand-50 p-3 font-mono text-xs break-all text-ink-700" data-testid="organization-card">{ownCard}</div>
              <div className="text-xs text-ink-500">Адрес: <span className="font-mono">{identityQ.data.destination}</span></div>
              <button className={btnSecondaryCls} onClick={() => navigator.clipboard.writeText(ownCard).then(() => toast('Визитка скопирована'))}>
                <Clipboard size={16} /> Копировать
              </button>
            </div>
          </div>
        )}
      </section>

      <section className={`${cardCls} overflow-hidden`}>
        <div className="border-b border-brand-100 px-5 py-4"><h3 className="font-semibold text-ink-900">Исходящие и квитанции</h3></div>
        <div className="divide-y divide-brand-100">
          {(outboxQ.data ?? []).map((item) => (
            <article key={item.transactionId} className="flex flex-col gap-2 p-5 sm:flex-row sm:items-center sm:justify-between" data-testid="interorg-outbox-item">
              <div className="min-w-0">
                <div className="flex items-center gap-2"><span className="font-semibold text-ink-900">{item.contact.name}</span><span className="rounded-full bg-brand-50 px-2 py-0.5 text-xs text-brand-700">{item.kind}</span></div>
                <p className="mt-1 truncate font-mono text-xs text-ink-500">{item.transactionId}</p>
                {item.acceptanceLedgerHash && <p className="mt-1 truncate font-mono text-[11px] text-ink-500" title={item.acceptanceLedgerHash}>Летопись получателя: {item.acceptanceLedgerHash}</p>}
              </div>
              {item.status === 'accepted' && item.acceptanceProofVerified
                ? <span className="inline-flex items-center gap-1 text-sm font-semibold text-teal"><Check size={16} /> Принято · device-proof проверен</span>
                : item.status === 'accepted'
                  ? <span className="text-sm font-semibold text-danger">Доказательство повреждено</span>
                : <span className="text-sm font-medium text-ink-500">Ожидает квитанцию</span>}
            </article>
          ))}
          {!outboxQ.isLoading && !(outboxQ.data ?? []).length && <p className="p-6 text-center text-sm text-ink-500">Исходящих транзакций пока нет</p>}
        </div>
      </section>

      <section className={`${cardCls} overflow-hidden`}>
        <div className="border-b border-brand-100 px-5 py-4"><h3 className="font-semibold text-ink-900">Доверенный каталог</h3></div>
        <div className="divide-y divide-brand-100">
          {(contactsQ.data ?? []).map((contact) => (
            <div key={contact.guid} className="flex flex-col gap-3 p-5 sm:flex-row sm:items-center sm:justify-between" data-testid="interorg-contact">
              <div className="min-w-0">
                <div className="flex items-center gap-2"><span className="font-semibold text-ink-900">{contact.name}</span><span className={`rounded-full px-2 py-0.5 text-xs ${contact.active ? 'bg-teal/10 text-teal' : 'bg-danger/10 text-danger'}`}>{contact.active ? 'доверен' : 'отозван'}</span></div>
                <p className="mt-1 truncate font-mono text-xs text-ink-500">{contact.remoteWorkspaceGuid}</p>
              </div>
              {contact.active && <button className={btnSecondaryCls} onClick={() => revoke.mutate({ workspaceId, guid: contact.guid })} disabled={revoke.isPending}><Ban size={16} /> Отозвать ключи</button>}
            </div>
          ))}
          {!contactsQ.isLoading && !(contactsQ.data ?? []).length && <p className="p-6 text-center text-sm text-ink-500">Контрагентов пока нет</p>}
        </div>
      </section>

      <section className={`${cardCls} p-5 sm:p-6`}>
        <h3 className="font-semibold text-ink-900">Добавить доверенную организацию</h3>
        <div className="mt-4 grid gap-3 lg:grid-cols-2">
          <input className={inputCls} placeholder="Название (необязательно)" value={contactName} onChange={(event) => setContactName(event.target.value)} />
          <input className={inputCls} placeholder="Вставьте строку из QR-визитки" value={cardText} onChange={(event) => setCardText(event.target.value)} data-testid="contact-card-input" />
        </div>
        <button className={`${btnPrimaryCls} mt-3`} onClick={addContact} disabled={!cardText.trim() || trust.isPending}>
          <ShieldCheck size={17} /> Проверить и доверять
        </button>
      </section>

      <section className={`${cardCls} p-5 sm:p-6`}>
        <h3 className="font-semibold text-ink-900">Отправить транзакцию</h3>
        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <select className={inputCls} value={selected} onChange={(event) => setSelected(event.target.value)} aria-label="Контрагент">
            <option value="">Выберите организацию</option>
            {(contactsQ.data ?? []).filter((item) => item.active).map((item) => <option key={item.guid} value={item.guid}>{item.name}</option>)}
          </select>
          <input className={inputCls} value={kind} onChange={(event) => setKind(event.target.value)} placeholder="Тип: invoice.offer" />
        </div>
        <textarea className="mt-3 min-h-28 w-full rounded-xl border border-brand-100 bg-surface p-4 text-sm outline-none focus:border-brand-600 focus:ring-[3px] focus:ring-[#5E629B22]" value={message} onChange={(event) => setMessage(event.target.value)} placeholder="Текст сообщения или условия сделки" />
        <button className={`${btnPrimaryCls} mt-3`} disabled={!selected || !kind.trim() || !message.trim() || send.isPending} onClick={() => send.mutate({ workspaceId, contactGuid: selected, transactionId: crypto.randomUUID(), kind: kind.trim(), body: { text: message.trim() } })}>
          <Send size={17} /> Подписать и отправить
        </button>
      </section>

      <section className={`${cardCls} overflow-hidden`}>
        <div className="border-b border-brand-100 px-5 py-4"><h3 className="font-semibold text-ink-900">Входящие</h3></div>
        <div className="divide-y divide-brand-100">
          {(inboxQ.data ?? []).map((item) => (
            <article key={item.envelopeId} className="flex flex-col gap-3 p-5 sm:flex-row sm:items-center sm:justify-between" data-testid="interorg-inbox-item">
              <div className="min-w-0">
                <div className="flex items-center gap-2"><span className="font-semibold text-ink-900">{item.contact.name}</span><span className="rounded-full bg-brand-50 px-2 py-0.5 text-xs text-brand-700">{item.kind}</span></div>
                <p className="mt-1 whitespace-pre-wrap text-sm text-ink-700">{typeof item.body === 'object' && item.body && 'text' in item.body ? String((item.body as { text: unknown }).text) : JSON.stringify(item.body)}</p>
              </div>
              {item.accepted ? <span className="inline-flex items-center gap-1 text-sm font-semibold text-teal"><Check size={16} /> Принято</span> : <button className={btnSecondaryCls} onClick={() => accept.mutate({ workspaceId, envelopeId: item.envelopeId })}><Check size={16} /> Принять</button>}
            </article>
          ))}
          {!inboxQ.isLoading && !(inboxQ.data ?? []).length && <p className="p-6 text-center text-sm text-ink-500">Входящих транзакций пока нет</p>}
        </div>
      </section>
    </div>
  )
}
