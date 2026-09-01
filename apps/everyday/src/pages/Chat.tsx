import { useEffect, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import { FileText, Loader2, MessageCircle, Paperclip, Send, ShieldCheck, X } from 'lucide-react'
import { format } from 'date-fns'
import { ru } from 'date-fns/locale'
import { trpc } from '@/providers/trpc'
import { useStore } from '@/lib/store'
import { cn } from '@/lib/utils'
import { BROWSER_FILE_LIMIT_BYTES, BROWSER_FILE_LIMIT_LABEL } from '@/lib/content-limits'

export default function Chat() {
  const { currentUser, workspace } = useStore()
  const utils = trpc.useUtils()
  const [text, setText] = useState('')
  const [files, setFiles] = useState<File[]>([])
  const [draftGuid, setDraftGuid] = useState(() => crypto.randomUUID())
  const [fileError, setFileError] = useState<string | null>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const fileRef = useRef<HTMLInputElement>(null)
  const listQ = trpc.chat.list.useQuery(
    { workspaceId: workspace?.id },
    { refetchInterval: 4000 },
  )
  const send = trpc.chat.send.useMutation({
    onSuccess: () => {
      setText('')
      setFiles([])
      setFileError(null)
      setDraftGuid(crypto.randomUUID())
      utils.chat.list.invalidate()
    },
  })
  const ingestContent = trpc.content.ingest.useMutation()

  const messages = listQ.data ?? []

  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight, behavior: 'smooth' })
  }, [messages.length])

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault()
    const value = text.trim()
    if ((!value && files.length === 0) || !workspace?.guid || send.isPending || ingestContent.isPending) return
    try {
      const messageGuid = draftGuid
      const attachments = await Promise.all(files.map(async (file) => {
        const dataUrl = await new Promise<string>((resolve, reject) => {
          const reader = new FileReader()
          reader.onload = () => resolve(String(reader.result))
          reader.onerror = () => reject(reader.error ?? new Error('Не удалось прочитать файл'))
          reader.readAsDataURL(file)
        })
        const uploaded = await ingestContent.mutateAsync({
          workspaceId: workspace.id,
          purpose: 'chat-attachment',
          messageGuid,
          dataUrl,
        })
        return { name: file.name, url: uploaded.url, mime: uploaded.mime }
      }))
      send.mutate({
        text: value,
        workspaceId: workspace.id,
        workspaceGuid: workspace.guid,
        messageGuid,
        attachments,
      })
    } catch {
      // Ошибка ingest уже отображается стандартным состоянием mutation.
    }
  }

  return (
    <div className="flex flex-col gap-4 h-[calc(100dvh-8rem)] lg:h-[calc(100dvh-6rem)]">
      <div>
        <h1 className="text-2xl lg:text-[28px] leading-9 font-bold tracking-[-0.01em] text-ink-900">
          Чат группы
        </h1>
        <p className="text-sm text-ink-500 mt-1">
          Сообщения хранятся на локальном узле «{workspace?.name ?? 'группа'}».
        </p>
      </div>

      <section className="flex-1 min-h-0 bg-surface rounded-card border border-brand-100/60 shadow-card flex flex-col">
        <div ref={listRef} className="flex-1 overflow-y-auto p-4 space-y-3">
          {listQ.isLoading && (
            <p className="text-sm text-ink-500 text-center py-10">Загружаем сообщения…</p>
          )}
          {!listQ.isLoading && messages.length === 0 && (
            <div className="flex flex-col items-center justify-center py-16 text-ink-300 gap-2">
              <MessageCircle size={36} strokeWidth={1.5} />
              <p className="text-sm font-semibold text-ink-500">Пока тихо — напишите первое сообщение</p>
            </div>
          )}
          {messages.map((m) => {
            const mine = m.userId === currentUser?.id
            return (
              <div key={m.id} className={cn('flex gap-2', mine && 'flex-row-reverse')}>
                {m.user?.avatarUrl ? (
                  <img
                    src={m.user.avatarUrl}
                    alt=""
                    className="w-8 h-8 rounded-full object-cover border border-brand-100 shrink-0"
                  />
                ) : (
                  <span className="w-8 h-8 rounded-full bg-brand-100/60 flex items-center justify-center text-xs font-semibold text-brand-700 shrink-0">
                    {(m.user?.fullName ?? '?').slice(0, 1)}
                  </span>
                )}
                <div className={cn('max-w-[80%]', mine && 'items-end')}>
                  <div className={cn('text-[12px] text-ink-500 mb-0.5', mine && 'text-right')}>
                    {m.user?.fullName ?? 'Участник'} ·{' '}
                    {format(new Date(m.createdAt), 'dd.MM HH:mm', { locale: ru })}
                    {m.ledgerVerified && (
                      <ShieldCheck
                        size={12}
                        className="ml-1 inline text-teal-dark"
                        aria-label="Подпись сообщения проверена"
                      />
                    )}
                  </div>
                  <div
                    className={cn(
                      'rounded-xl px-3 py-2 text-[15px] leading-[22px] whitespace-pre-wrap break-words',
                      mine ? 'bg-accent text-white' : 'bg-brand-50 text-ink-900'
                    )}
                  >
                    {m.text}
                  </div>
                  {!!m.attachments?.length && (
                    <div className="mt-1.5 flex flex-wrap gap-1.5">
                      {m.attachments.map((attachment) => (
                        <a key={`${m.guid}-${attachment.sha256}`} href={attachment.url} download={attachment.name} className="inline-flex max-w-full items-center gap-1.5 rounded-lg bg-brand-100/70 px-2.5 py-1.5 text-xs font-semibold text-brand-700 hover:bg-brand-100">
                          <FileText size={13} />
                          <span className="truncate">{attachment.name}</span>
                        </a>
                      ))}
                    </div>
                  )}
                </div>
              </div>
            )
          })}
        </div>
        <form onSubmit={onSubmit} className="border-t border-brand-100/60 p-3">
          {!!files.length && (
            <div className="mb-2 flex flex-wrap gap-2">
              {files.map((file, index) => (
                <span key={`${file.name}-${index}`} className="inline-flex max-w-[240px] items-center gap-1.5 rounded-lg bg-brand-50 px-2.5 py-1.5 text-xs font-semibold text-ink-700">
                  <Paperclip size={12} /><span className="truncate">{file.name}</span>
                  <button type="button" aria-label={`Убрать ${file.name}`} onClick={() => setFiles(current => current.filter((_, item) => item !== index))}><X size={12} /></button>
                </span>
              ))}
            </div>
          )}
          <div className="flex items-end gap-2">
          <input ref={fileRef} type="file" multiple className="hidden" onChange={(event) => {
            const candidates = Array.from(event.target.files ?? [])
            const selected = candidates.filter(file => file.size > 0 && file.size <= BROWSER_FILE_LIMIT_BYTES)
            const rejected = candidates.filter(file => file.size === 0 || file.size > BROWSER_FILE_LIMIT_BYTES)
            setFiles(current => [...current, ...selected].slice(0, 10))
            setFileError(rejected.length ? `Не добавлены: ${rejected.map(file => file.name).join(', ')}. Лимит ${BROWSER_FILE_LIMIT_LABEL}` : null)
            event.target.value = ''
          }} />
          <button type="button" aria-label="Прикрепить файлы" onClick={() => fileRef.current?.click()} className="h-11 w-11 shrink-0 rounded-xl border border-brand-100 text-brand-700 inline-flex items-center justify-center hover:bg-brand-50"><Paperclip size={17} /></button>
          <textarea
            value={text}
            onChange={(e) => setText(e.target.value)}
            rows={1}
            placeholder="Сообщение группе…"
            className="flex-1 rounded-xl border border-brand-100 bg-surface px-3 py-2.5 text-[15px] text-ink-900 placeholder:text-ink-300 focus:border-brand-600 focus:ring-[3px] focus:ring-brand-600/15 resize-none min-h-[44px] max-h-[120px]"
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                onSubmit(e)
              }
            }}
          />
          <button
            type="submit"
            disabled={(!text.trim() && files.length === 0) || send.isPending || ingestContent.isPending}
            className="h-11 px-4 rounded-xl bg-accent text-white text-sm font-semibold inline-flex items-center gap-2 hover:bg-accent-hover disabled:opacity-50"
          >
            {send.isPending || ingestContent.isPending ? <Loader2 size={16} className="animate-spin" /> : <Send size={16} />}
            Отправить
          </button>
          </div>
        </form>
        {send.error && (
          <p className="px-3 pb-3 text-xs text-danger" role="alert">
            {send.error.message}
          </p>
        )}
        {ingestContent.error && <p className="px-3 pb-3 text-xs text-danger" role="alert">{ingestContent.error.message}</p>}
        {fileError && <p className="px-3 pb-3 text-xs text-danger" role="alert">{fileError}</p>}
      </section>
    </div>
  )
}
