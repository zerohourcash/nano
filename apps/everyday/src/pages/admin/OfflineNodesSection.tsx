import { useState } from 'react'
import { GitBranch, Network, Radio, RefreshCw, ShieldAlert, ShieldCheck, Download, Upload, Trash2 } from 'lucide-react'
import { trpc } from '@/providers/trpc'
import { cn } from '@/lib/utils'
import { SectionHeader, btnPrimaryCls, btnSecondaryCls, cardCls, inputCls, useToast } from './ui'

function fmtMoment(iso: string): string {
  const date = new Date(iso)
  return Number.isNaN(date.getTime())
    ? iso
    : date.toLocaleString('ru-RU', { dateStyle: 'short', timeStyle: 'medium' })
}

export default function OfflineNodesSection() {
  const toast = useToast()
  const utils = trpc.useUtils()
  const statusQ = trpc.sync.status.useQuery(undefined, { refetchInterval: 8000 })
  const auditQ = trpc.sync.audit.useQuery(undefined, { refetchInterval: 30000 })
  const conflictsQ = trpc.sync.conflicts.useQuery(undefined, { refetchInterval: 8000 })
  const keysQ = trpc.sync.nodeKeys.useQuery(undefined, { refetchInterval: 8000 })
  const [peerUrl, setPeerUrl] = useState('')
  const [password, setPassword] = useState('')
  const addPeer = trpc.sync.addPeer.useMutation({
    onSuccess: () => {
      utils.sync.status.invalidate()
      toast('Узел добавлен, журнал подтянется сам')
      setPeerUrl('')
    },
    onError: (e) => toast(e.message, 'error'),
  })
  const removePeer = trpc.sync.removePeer.useMutation({
    onSuccess: () => {
      utils.sync.status.invalidate()
      toast('Узел удалён из mesh')
    },
    onError: (e) => toast(e.message, 'error'),
  })
  const pull = trpc.sync.pullNow.useMutation({
    onSuccess: () => {
      utils.sync.status.invalidate()
      toast('Синхронизация поставлена в очередь')
    },
    onError: (e) => toast(e.message, 'error'),
  })
  const resolve = trpc.sync.resolveConflict.useMutation({
    onSuccess: () => {
      utils.sync.conflicts.invalidate()
      utils.items.list.invalidate()
      toast('Конфликт закрыт')
    },
    onError: (e) => toast(e.message, 'error'),
  })
  const approveKey = trpc.sync.approveNodeKey.useMutation({
    onSuccess: () => { utils.sync.nodeKeys.invalidate(); pull.mutate({}); toast('Ключ узла одобрен') },
    onError: (e) => toast(e.message, 'error'),
  })
  const revokeKey = trpc.sync.revokeNodeKey.useMutation({
    onSuccess: () => { utils.sync.nodeKeys.invalidate(); toast('Доверие к ключу отозвано') },
    onError: (e) => toast(e.message, 'error'),
  })
  const exp = trpc.backup.export.useMutation({
    onSuccess: (blob) => {
      const a = document.createElement('a')
      a.href = URL.createObjectURL(new Blob([JSON.stringify(blob, null, 2)], { type: 'application/json' }))
      a.download = `meshkeeper-backup-${new Date().toISOString().slice(0, 10)}.json`
      a.click()
      toast('Шифроархив скачан — положите его в Google Drive')
    },
    onError: (e) => toast(e.message, 'error'),
  })
  const imp = trpc.backup.import.useMutation({
    onSuccess: (r) => toast(`Восстановлено: предметов ${r.items}, операций ${r.ops}`),
    onError: (e) => toast(e.message, 'error'),
  })

  const st = statusQ.data
  const isNode = st?.role === 'node' || st?.role === 'mesh'
  const peers = st?.peers ?? []
  const conflicts = (conflictsQ.data ?? []).filter((c) => c.status === 'open')

  const onImportFile = async (file: File) => {
    if (password.length < 12) {
      toast('Сначала введите пароль архива', 'error')
      return
    }
    const text = await file.text()
    const blob = JSON.parse(text)
    imp.mutate({ password, blob })
  }

  return (
    <div className="space-y-6">
      <SectionHeader title="Офлайн-узлы и синхронизация" />

      <section className={cardCls + ' p-5 space-y-3'}>
        <div className="flex items-center gap-2">
          <Radio size={18} className="text-brand-600" />
          <h3 className="text-[17px] font-semibold text-ink-900">Этот узел</h3>
        </div>
        <p className="text-sm text-ink-500">
          {isNode
            ? 'Mesh-узел: работает на своей базе и обменивается изменениями со всеми доступными peers.'
            : 'Принимающий узел: хранит локальную базу и готов к подключению участников mesh.'}
        </p>
        <div className="grid sm:grid-cols-2 gap-2 text-sm">
          <div>
            Роль: <b>{st?.role === 'mesh' ? 'mesh-узел' : isNode ? 'локальный узел' : 'принимающий узел'}</b>
          </div>
          <div>Имя: <b>{st?.name || '…'}</b></div>
          <div className="font-mono-num break-all">ID: {st?.nodeId || '…'}</div>
          <div className="sm:col-span-2 font-mono-num break-all">URL: {st?.url}</div>
          <div>Успешных обменов: <b>{st?.syncSuccesses ?? 0}</b></div>
          <div>
            Трафик: <b>{Math.round(((st?.bytesSent ?? 0) + (st?.bytesReceived ?? 0)) / 1024)} КБ</b>
          </div>
        </div>

        {st?.upstream && (
          <div className="rounded-xl border border-brand-100 p-3 space-y-2 text-sm">
            <div className="font-mono-num break-all">Сервер: {st?.upstream}</div>
            <div className="flex flex-wrap items-center gap-2">
              <span
                className={cn(
                  'inline-flex items-center rounded-full px-2.5 py-0.5 text-caption',
                  st?.lastError
                    ? 'bg-danger/10 text-danger'
                    : st?.lastSync
                      ? 'bg-teal/20 text-teal-dark'
                      : 'bg-brand-50 text-ink-500',
                )}
              >
                {st?.lastError ? 'нет связи' : st?.lastSync ? 'на связи' : 'ещё не синхронизировался'}
              </span>
              <span className="text-ink-500">
                {st?.lastSync ? `последний обмен: ${fmtMoment(st.lastSync)}` : ''}
              </span>
              <button
                className={btnSecondaryCls}
                disabled={pull.isPending}
                onClick={() => pull.mutate({})}
              >
                <RefreshCw size={14} /> Синхронизировать сейчас
              </button>
            </div>
            {st?.lastError && (
              <p className="text-danger">
                {st.lastError}. Узел продолжает работать на своей базе, обмен повторится автоматически.
              </p>
            )}
          </div>
        )}
      </section>

      <section className={cardCls + ' p-5 space-y-3'}>
        <div className="flex items-center gap-2">
          {auditQ.data?.healthy ? (
            <ShieldCheck size={18} className="text-teal-dark" />
          ) : (
            <ShieldAlert size={18} className="text-danger" />
          )}
          <h3 className="text-[17px] font-semibold text-ink-900">Целостность локальной копии</h3>
        </div>
        {auditQ.isPending ? (
          <p className="text-sm text-ink-500">Проверяем базу, подписи и полноту связей…</p>
        ) : (
          <>
            <p className={cn('text-sm font-semibold', auditQ.data?.healthy ? 'text-teal-dark' : 'text-danger')}>
              {auditQ.data?.healthy
                ? 'Локальная история и текущее состояние криптографически согласованы'
                : 'Обнаружена ошибка целостности — не используйте эту ноду для окончательного учёта'}
            </p>
            <div className="grid sm:grid-cols-3 gap-2 text-sm">
              <div>Операций: <b>{auditQ.data?.counts.history ?? 0}</b></div>
              <div>Предметов: <b>{auditQ.data?.counts.items ?? 0}</b></div>
              <div>Сообщений: <b>{auditQ.data?.counts.messages ?? 0}</b></div>
              <div>Проверено подписей: <b>{auditQ.data?.ledgerVerified ?? 0}</b></div>
              <div>Глав цепочек: <b>{auditQ.data?.ledgerHeads.length ?? 0}</b></div>
              <div>Потерянных связей: <b>{auditQ.data?.orphanHistory ?? 0}</b></div>
              <div>CAS-файлов: <b>{auditQ.data?.counts.blobs ?? 0}</b></div>
              <div>Недокачанных файлов: <b>{auditQ.data?.missingBlobs ?? 0}</b></div>
            </div>
            {(auditQ.data?.ledgerError || auditQ.data?.chatError || auditQ.data?.accountingError || auditQ.data?.snapshotError) && (
              <p className="text-sm text-danger break-all">
                {auditQ.data.ledgerError || auditQ.data.chatError || auditQ.data.accountingError || auditQ.data.snapshotError}
              </p>
            )}
            <div className="flex items-center gap-2">
              <button className={btnSecondaryCls} onClick={() => auditQ.refetch()} disabled={auditQ.isFetching}>
                <RefreshCw size={14} /> Проверить снова
              </button>
              <span className="text-[12px] text-ink-300">
                {auditQ.data?.checkedAt ? `проверено ${fmtMoment(auditQ.data.checkedAt)}` : ''}
              </span>
            </div>
            <p className="font-mono-num text-[11px] text-ink-300 break-all">
              Snapshot: {auditQ.data?.snapshotHash || 'нет'}
            </p>
          </>
        )}
      </section>

      <section className={cardCls + ' p-5 space-y-3'}>
        <div className="flex items-center gap-2"><ShieldCheck size={18} className="text-brand-600" /><h3 className="text-[17px] font-semibold text-ink-900">Ключи mesh-нод</h3></div>
        <p className="text-sm text-ink-500">Строгий режим: <b>{keysQ.data?.strict ? 'включён' : 'выключен'}</b>. Новый ключ не получает доступ к летописи до одобрения владельцем.</p>
        {(keysQ.data?.pending ?? []).map((key) => <div key={key.publicKey} className="rounded-xl border border-[#D8A928] p-3 text-sm space-y-2">
          <p><b>{key.nodeName || 'Новая нода'}</b> · {key.peerUrl || 'адрес не объявлен'}</p><p className="font-mono-num break-all text-[11px]">{key.publicKey}</p>
          <button className={btnPrimaryCls} onClick={() => approveKey.mutate({ publicKey: key.publicKey, label: key.nodeName || undefined })}>Одобрить ключ</button>
        </div>)}
        {(keysQ.data?.trusted ?? []).map((key) => <div key={key.publicKey} className="flex items-center justify-between gap-2 text-sm border-b border-brand-50 py-2">
          <span className="min-w-0"><b>{key.label || key.source}</b><span className="block font-mono-num truncate text-[11px] text-ink-300">{key.publicKey}</span></span>
          {key.source !== 'local' && <button className={btnSecondaryCls} onClick={() => revokeKey.mutate({ publicKey: key.publicKey })}>Отозвать</button>}
        </div>)}
      </section>

      <section className={cardCls + ' p-5 space-y-3'}>
        <div className="flex items-center gap-2">
          <Network size={18} className="text-brand-600" />
          <h3 className="text-[17px] font-semibold text-ink-900">
            {isNode ? 'Известные адреса' : 'Подключённые узлы'}
          </h3>
        </div>
        <div className="flex gap-2">
          <input
            className={inputCls}
            placeholder="https://trusted-sync.example.com"
            value={peerUrl}
            onChange={(e) => setPeerUrl(e.target.value)}
          />
          <button
            className={btnPrimaryCls}
            disabled={!peerUrl.trim() || addPeer.isPending}
            onClick={() => addPeer.mutate({ url: peerUrl.trim() })}
          >
            Добавить
          </button>
        </div>
        {peers.length === 0 && (
          <p className="text-sm text-ink-500">
            {isNode
              ? 'Адрес сервера задаётся переменной MESHKEEPER_UPSTREAM при запуске узла.'
              : 'Узлы появятся здесь после первого обмена. Общий секрет задаётся переменной MESHKEEPER_SYNC_TOKEN.'}
          </p>
        )}
        <ul className="space-y-2">
          {peers.map((p) => (
            <li key={p.id} className="rounded-xl border border-brand-100 px-3 py-2 text-sm flex items-center justify-between gap-2">
              <span className="min-w-0">
                <span className="font-semibold">{p.name || 'узел'}</span>
                <span className="block font-mono-num text-ink-500 truncate">{p.url}</span>
                <span className="text-[12px] text-ink-300">
                  sync {p.lastSync ?? 'ещё нет'} {p.lastError ? `· ${p.lastError}` : ''}
                </span>
              </span>
              <span className="flex gap-2">
                <button className={btnSecondaryCls} onClick={() => pull.mutate({})}>
                  <RefreshCw size={14} /> Сейчас
                </button>
                <button
                  className={btnSecondaryCls}
                  aria-label={`Удалить узел ${p.name || p.url}`}
                  onClick={() => removePeer.mutate({ url: p.url })}
                >
                  <Trash2 size={14} />
                </button>
              </span>
            </li>
          ))}
        </ul>
      </section>

      <section className={cardCls + ' p-5 space-y-3'}>
        <div className="flex items-center gap-2">
          <ShieldAlert size={18} className="text-[#A87C0F]" />
          <h3 className="text-[17px] font-semibold text-ink-900">Конфликты двойной выдачи</h3>
        </div>
        {conflicts.length === 0 && <p className="text-sm text-ink-500">Открытых конфликтов нет.</p>}
        {conflicts.map((c) => (
          <div key={c.id} className="rounded-xl border border-brand-100 p-3 space-y-2">
            <p className="text-sm font-semibold">{c.item?.internalId} {c.item?.title}</p>
            <p className="text-sm text-ink-500">{c.description}</p>
            <div className="flex gap-2">
              <button className={btnPrimaryCls} onClick={() => resolve.mutate({ id: c.id, responsibleUserId: null })}>
                Вернуть на склад
              </button>
            </div>
          </div>
        ))}
      </section>

      <section className={cardCls + ' p-5 space-y-3'}>
        <div className="flex items-center gap-2">
          <GitBranch size={18} className="text-brand-600" />
          <h3 className="text-[17px] font-semibold text-ink-900">Резервная копия (шифр)</h3>
        </div>
        <p className="text-sm text-ink-500">
          Архив защищён Argon2id и ChaCha20‑Poly1305. Облачное хранилище получает только шифротекст.
        </p>
        <input
          className={inputCls}
          type="password"
          placeholder="Пароль архива от 12 символов"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        <div className="flex flex-wrap gap-2">
          <button className={btnPrimaryCls} disabled={password.length < 12 || exp.isPending} onClick={() => exp.mutate({ password })}>
            <Download size={16} /> Скачать архив
          </button>
          <label className={btnSecondaryCls + ' cursor-pointer'}>
            <Upload size={16} /> Восстановить из файла
            <input
              type="file"
              accept="application/json"
              className="hidden"
              onChange={(e) => {
                const f = e.target.files?.[0]
                if (f) void onImportFile(f)
                e.target.value = ''
              }}
            />
          </label>
        </div>
      </section>
    </div>
  )
}
