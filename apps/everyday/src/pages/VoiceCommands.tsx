import { useEffect, useMemo, useState } from 'react'
import { Link, useNavigate, useSearchParams } from 'react-router'
import { CheckCircle2, Loader2, MessageCircle, Mic, MicOff, PackageSearch, ShieldCheck } from 'lucide-react'
import { trpc } from '@/providers/trpc'
import { useStore } from '@/lib/store'
import { normalizeVoiceMatch, parseVoiceCommand } from '@/lib/voice-command'
import type { VoiceIntent } from '@/lib/voice-command'

type RecognitionEvent = { results: ArrayLike<{ 0: { transcript: string } }> }
type Recognition = {
  lang: string
  interimResults: boolean
  continuous: boolean
  start: () => void
  stop: () => void
  onresult: ((event: RecognitionEvent) => void) | null
  onerror: (() => void) | null
  onend: (() => void) | null
}
type RecognitionConstructor = new () => Recognition

function recognitionConstructor(): RecognitionConstructor | undefined {
  const browser = window as unknown as {
    SpeechRecognition?: RecognitionConstructor
    webkitSpeechRecognition?: RecognitionConstructor
  }
  return browser.SpeechRecognition ?? browser.webkitSpeechRecognition
}

export default function VoiceCommands() {
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const { workspace } = useStore()
  const utils = trpc.useUtils()
  const initialCommand = searchParams.get('command')?.trim() ?? ''
  const [command, setCommand] = useState(initialCommand)
  const [intent, setIntent] = useState<VoiceIntent | null>(() => initialCommand ? parseVoiceCommand(initialCommand) : null)
  const [listening, setListening] = useState(false)
  const [recognition, setRecognition] = useState<Recognition | null>(null)
  const [message, setMessage] = useState('')
  const itemsQ = trpc.items.list.useQuery(
    { page: 1, limit: 500, workspaceId: workspace?.id },
    { enabled: Boolean(workspace?.id) },
  )
  const send = trpc.chat.send.useMutation({
    onSuccess: () => setMessage('Сообщение подписано и добавлено в локальную летопись'),
    onError: (error) => setMessage(error.message),
  })
  const returnItem = trpc.transfers.returnItem.useMutation({
    onSuccess: async () => {
      await utils.items.list.invalidate()
      setMessage('Возврат подписан и добавлен в локальную летопись')
    },
    onError: (error) => setMessage(error.message),
  })

  useEffect(() => () => recognition?.stop(), [recognition])

  const matches = useMemo(() => {
    if (!intent || !('query' in intent)) return []
    const needle = normalizeVoiceMatch(intent.query)
    return (itemsQ.data?.rows ?? []).filter((item) => {
      const searchable = normalizeVoiceMatch(`${item.title} ${item.internalId} ${item.serialNumber ?? ''}`)
      return searchable.includes(needle)
    }).slice(0, 5)
  }, [intent, itemsQ.data])

  const interpret = (value = command) => {
    const parsed = parseVoiceCommand(value)
    setIntent(parsed)
    setMessage('')
    if (parsed.kind === 'navigate') navigate(parsed.path)
  }

  const startListening = () => {
    const Constructor = recognitionConstructor()
    if (!Constructor) {
      setMessage('На этом устройстве нет системного распознавания речи. Введите команду текстом.')
      return
    }
    const next = new Constructor()
    next.lang = 'ru-RU'
    next.interimResults = false
    next.continuous = false
    next.onresult = (event) => {
      const value = event.results[0]?.[0]?.transcript?.trim() ?? ''
      setCommand(value)
      interpret(value)
    }
    next.onerror = () => setMessage('Не удалось распознать речь. Повторите или введите команду.')
    next.onend = () => setListening(false)
    setRecognition(next)
    setListening(true)
    next.start()
  }

  const selected = matches.length === 1 ? matches[0] : null
  const busy = send.isPending || returnItem.isPending

  return (
    <div className="mx-auto max-w-2xl space-y-5" data-testid="voice-commands">
      <div>
        <h1 className="text-2xl font-bold text-ink-900">Голосовые команды</h1>
        <p className="mt-1 text-sm text-ink-500">Речь распознаётся системным сервисом устройства. Любая транзакция сначала показывается для подтверждения.</p>
      </div>
      <section className="rounded-card border border-brand-100/60 bg-surface p-5 shadow-card space-y-4">
        <button type="button" onClick={listening ? () => recognition?.stop() : startListening}
          className="flex h-16 w-full items-center justify-center gap-3 rounded-2xl bg-accent text-base font-semibold text-white">
          {listening ? <MicOff size={24} /> : <Mic size={24} />}
          {listening ? 'Остановить запись' : 'Надиктовать команду'}
        </button>
        <div className="flex gap-2">
          <input value={command} onChange={(event) => setCommand(event.target.value)}
            onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); interpret() } }}
            placeholder="Например: отправь в чат смена закончена"
            aria-label="Текст голосовой команды"
            className="h-12 min-w-0 flex-1 rounded-xl border border-brand-100 px-3 text-sm" />
          <button type="button" onClick={() => interpret()} className="h-12 rounded-xl bg-brand-600 px-4 text-sm font-semibold text-white">Разобрать</button>
        </div>
        <div className="grid gap-2 text-xs text-ink-500 sm:grid-cols-2">
          <p>«Найди инструмент ВН-0042»</p><p>«Возьми перфоратор» → скан QR</p>
          <p>«Верни шуруповёрт Bosch»</p><p>«Отправь в чат: смена закончена»</p>
        </div>
      </section>

      {intent?.kind === 'unknown' && <p className="rounded-xl bg-warning-bg p-4 text-sm text-warning">Команда не распознана. Никаких действий не выполнено.</p>}
      {intent?.kind === 'chat' && (
        <section className="rounded-card border border-teal/40 bg-surface p-5 shadow-card space-y-3">
          <div className="flex items-center gap-2 font-semibold"><MessageCircle size={18} /> Сообщение в «{workspace?.name}»</div>
          <p className="rounded-xl bg-brand-50 p-3 text-sm">{intent.text}</p>
          <button disabled={busy || !workspace?.guid} onClick={() => workspace?.guid && send.mutate({ text: intent.text, workspaceId: workspace.id, workspaceGuid: workspace.guid, messageGuid: crypto.randomUUID(), attachments: [] })}
            className="flex h-11 w-full items-center justify-center gap-2 rounded-xl bg-accent text-sm font-semibold text-white disabled:opacity-50">
            {busy ? <Loader2 className="animate-spin" size={16} /> : <ShieldCheck size={16} />} Подписать и отправить
          </button>
        </section>
      )}
      {intent && 'query' in intent && (
        <section className="rounded-card border border-brand-100/60 bg-surface p-5 shadow-card space-y-3">
          <div className="flex items-center gap-2 font-semibold"><PackageSearch size={18} /> Найдено: {matches.length}</div>
          {matches.map((item) => <Link key={item.id} to={`/tool/${item.id}`} className="block rounded-xl border border-brand-100 p-3 text-sm hover:bg-brand-50"><b>{item.title}</b> · {item.internalId}</Link>)}
          {matches.length === 0 && !itemsQ.isLoading && <p className="text-sm text-warning">ТМЦ не найдено. Никаких действий не выполнено.</p>}
          {matches.length > 1 && <p className="text-sm text-warning">Уточните название или внутренний номер — неоднозначная команда не выполняется.</p>}
          {selected && intent.kind === 'find' && <button onClick={() => navigate(`/tool/${selected.id}`)} className="h-11 w-full rounded-xl bg-brand-600 text-sm font-semibold text-white">Открыть карточку</button>}
          {selected && intent.kind === 'checkout' && <button onClick={() => navigate('/scan')} className="h-11 w-full rounded-xl bg-accent text-sm font-semibold text-white">Перейти к обязательному сканированию QR</button>}
          {selected && intent.kind === 'return' && <button disabled={busy || !selected.responsibleUserId} onClick={() => returnItem.mutate({ itemId: selected.id, comment: `Голосовая команда: ${command}` })}
            className="flex h-11 w-full items-center justify-center gap-2 rounded-xl bg-accent text-sm font-semibold text-white disabled:opacity-50">
            {busy ? <Loader2 className="animate-spin" size={16} /> : <CheckCircle2 size={16} />} Подтвердить возврат
          </button>}
        </section>
      )}
      {message && <p role="status" className="rounded-xl bg-info-bg p-4 text-sm text-ink-900">{message}</p>}
    </div>
  )
}
