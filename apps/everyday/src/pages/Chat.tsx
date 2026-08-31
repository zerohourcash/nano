import { useEffect, useRef, useState } from 'react'
import type { FormEvent } from 'react'
import { Loader2, MessageCircle, Send } from 'lucide-react'
import { format } from 'date-fns'
import { ru } from 'date-fns/locale'
import { trpc } from '@/providers/trpc'
import { useStore } from '@/lib/store'
import { cn } from '@/lib/utils'

export default function Chat() {
  const { currentUser, workspace } = useStore()
  const utils = trpc.useUtils()
  const [text, setText] = useState('')
  const listRef = useRef<HTMLDivElement>(null)
  const listQ = trpc.chat.list.useQuery(
    { workspaceId: workspace?.id },
    { refetchInterval: 4000 },
  )
  const send = trpc.chat.send.useMutation({
    onSuccess: () => {
      setText('')
      utils.chat.list.invalidate()
    },
  })

  const messages = listQ.data ?? []

  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight, behavior: 'smooth' })
  }, [messages.length])

  const onSubmit = (e: FormEvent) => {
    e.preventDefault()
    const value = text.trim()
    if (!value || send.isPending) return
    send.mutate({ text: value, workspaceId: workspace?.id })
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
                  </div>
                  <div
                    className={cn(
                      'rounded-xl px-3 py-2 text-[15px] leading-[22px] whitespace-pre-wrap break-words',
                      mine ? 'bg-accent text-white' : 'bg-brand-50 text-ink-900'
                    )}
                  >
                    {m.text}
                  </div>
                </div>
              </div>
            )
          })}
        </div>
        <form onSubmit={onSubmit} className="border-t border-brand-100/60 p-3 flex items-end gap-2">
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
            disabled={!text.trim() || send.isPending}
            className="h-11 px-4 rounded-xl bg-accent text-white text-sm font-semibold inline-flex items-center gap-2 hover:bg-accent-hover disabled:opacity-50"
          >
            {send.isPending ? <Loader2 size={16} className="animate-spin" /> : <Send size={16} />}
            Отправить
          </button>
        </form>
      </section>
    </div>
  )
}
