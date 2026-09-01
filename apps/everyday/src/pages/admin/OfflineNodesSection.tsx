import { useEffect, useState } from 'react'
import { Database, GitBranch, Network, Radio, RefreshCw, ShieldAlert, ShieldCheck, Download, Upload, Trash2 } from 'lucide-react'
import { trpc } from '@/providers/trpc'
import { cn } from '@/lib/utils'
import { SectionHeader, btnPrimaryCls, btnSecondaryCls, cardCls, inputCls, useToast } from './ui'

function fmtMoment(iso: string): string {
  const date = new Date(iso)
  return Number.isNaN(date.getTime())
    ? iso
    : date.toLocaleString('ru-RU', { dateStyle: 'short', timeStyle: 'medium' })
}

function fmtBytes(value: number): string {
  if (value < 1024) return `${value} Б`
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} КБ`
  if (value < 1024 * 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} МБ`
  return `${(value / 1024 / 1024 / 1024).toFixed(1)} ГБ`
}

export default function OfflineNodesSection() {
  const toast = useToast()
  const utils = trpc.useUtils()
  const statusQ = trpc.sync.status.useQuery(undefined, { refetchInterval: 8000 })
  const auditQ = trpc.sync.audit.useQuery(undefined, { refetchInterval: 30000 })
  const conflictsQ = trpc.sync.conflicts.useQuery(undefined, { refetchInterval: 8000 })
  const keysQ = trpc.sync.nodeKeys.useQuery(undefined, { refetchInterval: 8000 })
  const diagnosticsQ = trpc.sync.diagnostics.useQuery(undefined, { refetchInterval: 8000 })
  const contentQ = trpc.content.status.useQuery(undefined, { refetchInterval: 8000 })
  const bundleQ = trpc.sync.exportBundle.useQuery(undefined, { enabled: false })
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
  const clearDiagnostics = trpc.sync.clearDiagnostics.useMutation({
    onSuccess: () => { diagnosticsQ.refetch(); toast('Закрытые записи диагностики удалены') },
    onError: (e) => toast(e.message, 'error'),
  })
  const importBundle = trpc.sync.importBundle.useMutation({
    onSuccess: (result) => {
      utils.invalidate()
      toast(`Пакет проверен: импортировано ${result.imported ?? 0}, конфликтов ${result.conflicts ?? 0}`)
    },
    onError: (e) => toast(e.message, 'error'),
  })
  const setContentMode = trpc.content.setMode.useMutation({
    onSuccess: (state) => {
      utils.content.status.setData(undefined, state)
      pull.mutate({})
      toast(state.mode === 'full' ? 'Полная нода включена: начата докачка CAS-файлов' : 'Режим хранения изменён')
    },
    onError: (e) => toast(e.message, 'error'),
  })

  useEffect(() => {
    const consumeNativeBundle = () => {
      const bridge = (window as Window & {
        MeshKeeperNative?: { takePendingSyncBundle?: () => string }
      }).MeshKeeperNative
      const raw = bridge?.takePendingSyncBundle?.()
      if (!raw) return
      try {
        importBundle.mutate({ bundle: JSON.parse(raw) })
      } catch {
        toast('Android передал некорректный пакет Everyday', 'error')
      }
    }
    consumeNativeBundle()
    window.addEventListener('meshkeeper-native-bundle', consumeNativeBundle)
    return () => window.removeEventListener('meshkeeper-native-bundle', consumeNativeBundle)
  }, [importBundle, toast])
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
  const diagnostics = diagnosticsQ.data?.events ?? []

  const onImportFile = async (file: File) => {
    if (password.length < 12) {
      toast('Сначала введите пароль архива', 'error')
      return
    }
    const text = await file.text()
    const blob = JSON.parse(text)
    imp.mutate({ password, blob })
  }

  const exportTransportBundle = async () => {
    const result = await bundleQ.refetch()
    if (!result.data) {
      toast(result.error?.message ?? 'Не удалось собрать пакет', 'error')
      return
    }
    const file = new File(
      [JSON.stringify(result.data)],
      `everyday-sync-${new Date().toISOString().replace(/[:.]/g, '-')}.json`,
      { type: 'application/vnd.everyday.sync+json' },
    )
    if (navigator.canShare?.({ files: [file] })) {
      try {
        await navigator.share({ files: [file], title: 'Everyday: зашифрованный пакет синхронизации' })
        toast('Пакет передан системному меню обмена')
        return
      } catch (error) {
        if (error instanceof DOMException && error.name === 'AbortError') return
      }
    }
    const url = URL.createObjectURL(file)
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = file.name
    anchor.click()
    URL.revokeObjectURL(url)
    toast('Пакет скачан — передайте его Bluetooth, Wi‑Fi Direct или USB')
  }

  const importTransportFile = async (file: File) => {
    if (file.size > 30 * 1024 * 1024) {
      toast('Пакет превышает лимит 30 МБ', 'error')
      return
    }
    try {
      importBundle.mutate({ bundle: JSON.parse(await file.text()) })
    } catch {
      toast('Файл не является корректным JSON-пакетом Everyday', 'error')
    }
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
          <div className="sm:col-span-2">
            Scope организаций: <b>{st?.workspaceScopeMode === 'restricted' ? `${st.workspaceScope.length} разрешено` : 'вся база (доверенная нода)'}</b>
          </div>
          {st?.workspaceScopeMode === 'restricted' && (
            <div className="sm:col-span-2 break-all font-mono-num text-[11px] text-ink-500">
              {st.workspaceScope.length ? st.workspaceScope.join(', ') : 'Ни одна организация не разрешена: синхронизация закрыта'}
            </div>
          )}
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

      <section className={cardCls + ' p-5 space-y-3'} data-testid="content-node-mode">
        <div className="flex items-center gap-2">
          <Database size={18} className="text-brand-600" />
          <h3 className="text-[17px] font-semibold text-ink-900">Хранение файлов на этой ноде</h3>
        </div>
        <p className="text-sm text-ink-500">
          Подписанная летопись, транзакции и текст синхронизируются всегда. Режим определяет только хранение фотографий и документов по их CAS-хэшам.
        </p>
        <div className="grid gap-2 sm:grid-cols-3">
          {([
            ['metadata', 'Только летопись', 'Не хранить чужие тяжёлые файлы'],
            ['smart', 'Умный режим', 'Хранить свои файлы и нужные превью'],
            ['full', 'Полная нода', 'Докачать и проверять все известные файлы'],
          ] as const).map(([mode, title, description]) => (
            <button
              key={mode}
              type="button"
              className={cn('rounded-xl border p-3 text-left', contentQ.data?.mode === mode ? 'border-brand-600 bg-brand-50' : 'border-brand-100')}
              disabled={setContentMode.isPending}
              onClick={() => setContentMode.mutate({ mode })}
            >
              <span className="block text-sm font-semibold">{title}</span>
              <span className="block text-[12px] text-ink-500">{description}</span>
            </button>
          ))}
        </div>
        <div className="grid gap-2 text-sm sm:grid-cols-3">
          <div>Локально: <b>{contentQ.data?.blobs ?? 0}</b> · {fmtBytes(contentQ.data?.bytes ?? 0)}</div>
          <div>В каталоге: <b>{contentQ.data?.catalogEntries ?? 0}</b> · {fmtBytes(contentQ.data?.catalogBytes ?? 0)}</div>
          <div>Отсутствует: <b>{contentQ.data?.missing ?? 0}</b> · {fmtBytes(contentQ.data?.missingBytes ?? 0)}</div>
          <div>Докачивается: <b>{contentQ.data?.pending ?? 0}</b></div>
          <div>Закреплено: <b>{contentQ.data?.pinned ?? 0}</b></div>
          <div>Известных провайдеров: <b>{contentQ.data?.providers ?? 0}</b></div>
        </div>
        {contentQ.data?.mode === 'full' && (contentQ.data?.missing ?? 0) === 0 && (
          <p className="rounded-xl bg-teal/10 p-3 text-sm text-teal-dark">Полная копия всех известных CAS-файлов сохранена на этой ноде.</p>
        )}
        {contentQ.data?.mode === 'full' && (contentQ.data?.missing ?? 0) > 0 && (
          <p className="rounded-xl bg-[#FFFDF2] p-3 text-sm text-[#80600C]">Нода продолжит докачку с доступных peers после восстановления связи. Каждый файл принимается только после проверки SHA-256.</p>
        )}
      </section>

      <section className={cardCls + ' p-5 space-y-3'} data-testid="node-diagnostics">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <ShieldAlert size={18} className={diagnosticsQ.data?.unresolved ? 'text-danger' : 'text-teal-dark'} />
            <h3 className="text-[17px] font-semibold text-ink-900">Диагностика ноды</h3>
          </div>
          <button
            className={btnSecondaryCls}
            disabled={clearDiagnostics.isPending}
            onClick={() => clearDiagnostics.mutate({})}
          >
            <Trash2 size={15} /> Очистить закрытые
          </button>
        </div>
        <p className="text-sm text-ink-500">
          Активных проблем: <b>{diagnosticsQ.data?.unresolved ?? 0}</b>. Повторяющиеся ошибки объединяются и не раздувают базу.
        </p>
        {diagnostics.length === 0 && (
          <p className="rounded-xl bg-teal/10 p-3 text-sm text-teal-dark">Ошибок ноды пока не зафиксировано.</p>
        )}
        <div className="max-h-80 space-y-2 overflow-auto">
          {diagnostics.map((event) => (
            <div key={event.id} className={cn(
              'rounded-xl border p-3 text-sm',
              event.resolvedAt ? 'border-brand-100 opacity-60' : event.severity === 'critical' || event.severity === 'error' ? 'border-danger/40 bg-danger/5' : 'border-[#E8D48A] bg-[#FFFDF2]',
            )}>
              <div className="flex flex-wrap items-center justify-between gap-2">
                <b>{event.component} · {event.code}</b>
                <span className="font-mono-num text-[11px] text-ink-300">{fmtMoment(event.lastAt)} · ×{event.count}</span>
              </div>
              <p className="mt-1 break-words text-ink-700">{event.message}</p>
              {event.context?.peer && <p className="mt-1 break-all font-mono-num text-[11px] text-ink-500">Peer: {event.context.peer}</p>}
              {event.resolvedAt && <p className="mt-1 text-[11px] text-teal-dark">Закрыто {fmtMoment(event.resolvedAt)}</p>}
            </div>
          ))}
        </div>
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
              <div>Страниц знаний: <b>{auditQ.data?.counts.knowledgePages ?? 0}</b></div>
              <div>Ревизий знаний: <b>{auditQ.data?.counts.knowledgeRevisions ?? 0}</b></div>
              <div>Недокачанных файлов: <b>{auditQ.data?.missingBlobs ?? 0}</b></div>
            </div>
            {(auditQ.data?.ledgerError || auditQ.data?.chatError || auditQ.data?.accountingError || auditQ.data?.knowledgeError || auditQ.data?.snapshotError) && (
              <p className="text-sm text-danger break-all">
                {auditQ.data.ledgerError || auditQ.data.chatError || auditQ.data.accountingError || auditQ.data.knowledgeError || auditQ.data.snapshotError}
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
          <Upload size={18} className="text-brand-600" />
          <h3 className="text-[17px] font-semibold text-ink-900">Обмен без прямого соединения</h3>
        </div>
        <p className="text-sm text-ink-500">
          Зашифрованный и подписанный пакет содержит транзакции, текст и CAS-manifests, но не тяжёлые файлы. Передайте его через системный Bluetooth Share, Wi‑Fi Direct, AirDrop, USB или любой доступный канал.
        </p>
        <div className="flex flex-wrap gap-2">
          <button className={btnPrimaryCls} disabled={bundleQ.isFetching} onClick={() => void exportTransportBundle()}>
            <Download size={16} /> Передать пакет
          </button>
          <label className={btnSecondaryCls + ' cursor-pointer'}>
            <Upload size={16} /> Принять пакет
            <input type="file" accept=".json,application/json,application/vnd.everyday.sync+json" className="hidden" onChange={(event) => {
              const file = event.target.files?.[0]
              if (file) void importTransportFile(file)
              event.target.value = ''
            }} />
          </label>
        </div>
        <p className="text-[12px] text-ink-300">Посредник не видит содержимое; AEAD, подпись и хэш проверяются до изменения локальной базы.</p>
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
