import { useState } from 'react'
import { GitBranch, Network, Radio, RefreshCw, ShieldAlert, Download, Upload } from 'lucide-react'
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
  const conflictsQ = trpc.sync.conflicts.useQuery(undefined, { refetchInterval: 8000 })
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
  const isNode = st?.role === 'node'
  const peers = st?.peers ?? []
  const conflicts = (conflictsQ.data ?? []).filter((c) => c.status === 'open')

  const onImportFile = async (file: File) => {
    if (password.length < 8) {
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
            ? 'Локальный узел: работает на своей базе и обменивается изменениями с сервером, когда есть связь.'
            : 'Центральный сервер: хранит общую базу и принимает обмен от локальных узлов.'}
        </p>
        <div className="grid sm:grid-cols-2 gap-2 text-sm">
          <div>
            Роль: <b>{isNode ? 'локальный узел' : 'сервер'}</b>
          </div>
          <div>Имя: <b>{st?.name || '…'}</b></div>
          <div className="font-mono-num break-all">ID: {st?.nodeId || '…'}</div>
          <div className="sm:col-span-2 font-mono-num break-all">URL: {st?.url}</div>
        </div>

        {isNode && (
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
              {isNode && (
                <button className={btnSecondaryCls} onClick={() => pull.mutate({})}>
                  <RefreshCw size={14} /> Сейчас
                </button>
              )}
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
          placeholder="Пароль архива от 8 символов"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
        />
        <div className="flex flex-wrap gap-2">
          <button className={btnPrimaryCls} disabled={password.length < 8 || exp.isPending} onClick={() => exp.mutate({ password })}>
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
