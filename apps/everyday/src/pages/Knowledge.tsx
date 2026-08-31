import { useRef, useState } from 'react'
import type { ChangeEvent, FormEvent } from 'react'
import { AlertTriangle, BookOpen, FilePlus2, Loader2, Paperclip, Plus, Save, ShieldCheck, X } from 'lucide-react'
import { trpc } from '@/providers/trpc'
import { useStore } from '@/lib/store'
import { toast } from 'sonner'
import { cn } from '@/lib/utils'

type Visibility = 'members' | 'accounting' | 'managers'
type PageSummary = {
  guid: string
  slug: string
  title: string
  visibility: Visibility
  currentRevisionGuid: string | null
  hasConflict: boolean
  headCount: number
  updatedAt: string | null
}
type Attachment = { name: string; url: string; mime?: string | null }
type Revision = {
  guid: string
  parentGuid: string | null
  authorName: string | null
  content: string
  attachments: Attachment[]
  revisionHash: string
  createdAt: string
}
type PageDetail = PageSummary & {
  headGuids: string[]
  current: Revision | null
}

function fileAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result))
    reader.onerror = () => reject(reader.error ?? new Error('Не удалось прочитать файл'))
    reader.readAsDataURL(file)
  })
}

function slugify(value: string) {
  return value
    .toLocaleLowerCase('ru')
    .trim()
    .replace(/[^a-zа-яё0-9]+/gi, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 120)
}

export default function Knowledge() {
  const { workspace } = useStore()
  const utils = trpc.useUtils()
  const [selectedSlug, setSelectedSlug] = useState('')
  const [editing, setEditing] = useState(false)
  const [title, setTitle] = useState('')
  const [slug, setSlug] = useState('')
  const [content, setContent] = useState('')
  const [visibility, setVisibility] = useState<Visibility>('members')
  const [attachments, setAttachments] = useState<Attachment[]>([])
  const fileRef = useRef<HTMLInputElement>(null)

  const listQ = trpc.knowledge.list.useQuery(
    { workspaceId: workspace?.id ?? 0 },
    { enabled: !!workspace?.id, refetchInterval: 10_000 },
  )
  const pages = (listQ.data ?? []) as unknown as PageSummary[]
  const effectiveSlug = selectedSlug || pages[0]?.slug || ''

  const pageQ = trpc.knowledge.bySlug.useQuery(
    { workspaceId: workspace?.id ?? 0, slug: effectiveSlug || '_' },
    { enabled: !!workspace?.id && !!effectiveSlug },
  )
  const page = (pageQ.data ?? null) as unknown as PageDetail | null

  const save = trpc.knowledge.save.useMutation({
    onSuccess: (raw) => {
      const saved = raw as unknown as PageDetail
      setSelectedSlug(saved.slug)
      setEditing(false)
      utils.knowledge.list.invalidate()
      utils.knowledge.bySlug.invalidate()
      toast.success('Подписанная ревизия сохранена локально')
    },
    onError: (error) => toast.error(error.message),
  })

  const startCreate = () => {
    setTitle('')
    setSlug('')
    setContent('')
    setVisibility('members')
    setAttachments([])
    setEditing(true)
  }

  const startEdit = () => {
    if (!page) return
    setTitle(page.title)
    setSlug(page.slug)
    setContent(page.current?.content ?? '')
    setVisibility(page.visibility)
    setAttachments(page.current?.attachments ?? [])
    setEditing(true)
  }

  const addFiles = async (event: ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(event.target.files ?? [])
    event.target.value = ''
    if (attachments.length + files.length > 20) {
      toast.error('На одну ревизию можно приложить не более 20 файлов')
      return
    }
    try {
      const next = await Promise.all(files.map(async (file) => {
        if (file.size > 32 * 1024 * 1024) throw new Error(`${file.name}: максимум 32 МБ`)
        return { name: file.name, mime: file.type || undefined, url: await fileAsDataUrl(file) }
      }))
      setAttachments((current) => [...current, ...next])
    } catch (error) {
      toast.error(error instanceof Error ? error.message : 'Не удалось приложить файл')
    }
  }

  const onSubmit = (event: FormEvent) => {
    event.preventDefault()
    if (!workspace || !title.trim() || !slug.trim()) return
    save.mutate({
      workspaceId: workspace.id,
      title: title.trim(),
      slug: slugify(slug),
      content,
      visibility,
      parentRevisionGuid: page?.slug === slugify(slug) ? page.currentRevisionGuid ?? undefined : undefined,
      attachments: attachments.map(({ name, url, mime }) => ({ name, url, ...(mime ? { mime } : {}) })),
    })
  }

  const revisionLabel = page?.current?.revisionHash?.slice(0, 12)

  return (
    <div className="space-y-4" data-testid="knowledge-page">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-2xl lg:text-[28px] leading-9 font-bold text-ink-900">База знаний</h1>
          <p className="mt-1 text-sm text-ink-500">Подписанная локальная wiki организации «{workspace?.name ?? '—'}»</p>
        </div>
        <button onClick={startCreate} className="inline-flex items-center gap-2 rounded-xl bg-accent px-4 py-2.5 text-sm font-semibold text-white hover:opacity-90">
          <Plus size={17} /> Новая страница
        </button>
      </div>

      <div className="grid min-h-[560px] gap-4 lg:grid-cols-[280px_minmax(0,1fr)]">
        <aside className="rounded-card border border-brand-100/60 bg-surface p-2 shadow-card">
          {listQ.isLoading && <p className="p-4 text-sm text-ink-500">Загружаем оглавление…</p>}
          {!listQ.isLoading && pages.length === 0 && (
            <div className="flex flex-col items-center gap-2 px-4 py-12 text-center text-ink-300">
              <BookOpen size={34} />
              <p className="text-sm font-semibold text-ink-500">Пока нет страниц</p>
            </div>
          )}
          <div className="space-y-1">
            {pages.map((item) => (
              <button key={item.guid} onClick={() => { setSelectedSlug(item.slug); setEditing(false) }} className={cn('w-full rounded-xl px-3 py-2.5 text-left', item.slug === effectiveSlug ? 'bg-brand-50 text-brand-700' : 'hover:bg-brand-50/60')}>
                <span className="block truncate text-sm font-semibold">{item.title}</span>
                <span className="mt-0.5 flex items-center gap-1 text-[11px] text-ink-500">
                  {item.hasConflict && <AlertTriangle size={12} className="text-warning" />} {item.slug}
                </span>
              </button>
            ))}
          </div>
        </aside>

        <section className="rounded-card border border-brand-100/60 bg-surface p-4 sm:p-6 shadow-card">
          {editing ? (
            <form onSubmit={onSubmit} className="space-y-4" data-testid="knowledge-editor">
              <div className="grid gap-3 sm:grid-cols-2">
                <label className="text-sm font-semibold text-ink-700">Название
                  <input value={title} onChange={(e) => { setTitle(e.target.value); if (!page) setSlug(slugify(e.target.value)) }} maxLength={200} required className="mt-1 w-full rounded-xl border border-brand-100 px-3 py-2.5 font-normal outline-none focus:border-accent" />
                </label>
                <label className="text-sm font-semibold text-ink-700">Адрес страницы
                  <input value={slug} onChange={(e) => setSlug(slugify(e.target.value))} maxLength={120} required disabled={!!page && page.slug === slug} className="mt-1 w-full rounded-xl border border-brand-100 px-3 py-2.5 font-mono text-sm font-normal outline-none focus:border-accent disabled:bg-brand-50" />
                </label>
              </div>
              <label className="block text-sm font-semibold text-ink-700">Доступ
                <select value={visibility} onChange={(e) => setVisibility(e.target.value as Visibility)} className="mt-1 w-full rounded-xl border border-brand-100 px-3 py-2.5 font-normal outline-none focus:border-accent">
                  <option value="members">Все участники</option>
                  <option value="accounting">Бухгалтерия и аудиторы</option>
                  <option value="managers">Руководители</option>
                </select>
              </label>
              <label className="block text-sm font-semibold text-ink-700">Текст
                <textarea value={content} onChange={(e) => setContent(e.target.value)} rows={14} maxLength={2_000_000} className="mt-1 w-full resize-y rounded-xl border border-brand-100 px-3 py-2.5 font-mono text-sm font-normal leading-6 outline-none focus:border-accent" placeholder="# Инструкция&#10;&#10;Текст работает без интернета…" />
              </label>
              <div className="space-y-2">
                <input ref={fileRef} type="file" multiple className="hidden" onChange={addFiles} />
                <button type="button" onClick={() => fileRef.current?.click()} className="inline-flex items-center gap-2 rounded-xl border border-brand-100 px-3 py-2 text-sm font-semibold text-brand-700 hover:bg-brand-50"><Paperclip size={16} /> Приложить файл</button>
                {attachments.map((file, index) => (
                  <div key={`${file.name}-${index}`} className="flex items-center gap-2 rounded-xl bg-brand-50 px-3 py-2 text-sm">
                    <FilePlus2 size={15} className="text-brand-600" /><span className="min-w-0 flex-1 truncate">{file.name}</span>
                    <button type="button" aria-label={`Удалить ${file.name}`} onClick={() => setAttachments((rows) => rows.filter((_, i) => i !== index))}><X size={16} /></button>
                  </div>
                ))}
              </div>
              <div className="flex flex-wrap gap-2">
                <button disabled={save.isPending} className="inline-flex items-center gap-2 rounded-xl bg-accent px-4 py-2.5 text-sm font-semibold text-white disabled:opacity-50">
                  {save.isPending ? <Loader2 size={16} className="animate-spin" /> : <Save size={16} />} Подписать ревизию
                </button>
                <button type="button" onClick={() => setEditing(false)} className="rounded-xl border border-brand-100 px-4 py-2.5 text-sm font-semibold text-ink-700">Отмена</button>
              </div>
            </form>
          ) : pageQ.isLoading ? (
            <p className="py-16 text-center text-sm text-ink-500">Загружаем страницу…</p>
          ) : page ? (
            <article data-testid="knowledge-viewer">
              <div className="flex flex-wrap items-start justify-between gap-3 border-b border-brand-100 pb-4">
                <div>
                  <h2 className="text-2xl font-bold text-ink-900">{page.title}</h2>
                  <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-ink-500">
                    <span className="font-mono">{page.slug}</span><span>·</span><ShieldCheck size={13} className="text-teal-dark" /><span>ревизия {revisionLabel}</span>
                  </div>
                </div>
                <button onClick={startEdit} className="rounded-xl border border-brand-100 px-4 py-2 text-sm font-semibold text-brand-700 hover:bg-brand-50">Редактировать</button>
              </div>
              {page.hasConflict && (
                <div className="my-4 flex gap-3 rounded-xl border border-amber-200 bg-amber-50 p-3 text-sm text-amber-900" role="alert">
                  <AlertTriangle className="shrink-0" size={18} /><div><b>Найдены параллельные офлайн-правки.</b> Сохранены {page.headGuids.length} ветки; показана детерминированно выбранная версия.</div>
                </div>
              )}
              <div className="min-h-48 whitespace-pre-wrap break-words py-5 text-[15px] leading-7 text-ink-900">{page.current?.content || 'Пустая страница'}</div>
              {!!page.current?.attachments.length && (
                <div className="border-t border-brand-100 pt-4"><h3 className="mb-2 text-sm font-bold text-ink-700">Вложения</h3><div className="flex flex-wrap gap-2">
                  {page.current.attachments.map((file, index) => <a key={`${file.name}-${index}`} href={file.url} download={file.name} className="inline-flex items-center gap-2 rounded-xl bg-brand-50 px-3 py-2 text-sm font-semibold text-brand-700 hover:bg-brand-100"><Paperclip size={15} />{file.name}</a>)}
                </div></div>
              )}
              <p className="mt-5 text-xs text-ink-300">Автор: {page.current?.authorName ?? 'участник'} · {page.current?.createdAt ? new Date(page.current.createdAt).toLocaleString('ru-RU') : '—'}</p>
            </article>
          ) : (
            <div className="flex flex-col items-center gap-3 py-20 text-center"><BookOpen size={42} className="text-ink-300" /><p className="text-sm font-semibold text-ink-500">Выберите страницу или создайте новую</p></div>
          )}
        </section>
      </div>
    </div>
  )
}
