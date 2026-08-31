import { useEffect, useMemo, useState } from 'react'
import { Reorder } from 'framer-motion'
import { GripVertical, MoreHorizontal, Pencil, Plus, Tag, Trash2 } from 'lucide-react'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { trpc } from '@/providers/trpc'
import { cn } from '@/lib/utils'
import type { DictEntryDto, DictKind } from './types'
import {
  Field,
  Modal,
  SectionHeader,
  btnPrimaryCls,
  btnSecondaryCls,
  cardCls,
  inputCls,
  plural,
  useToast,
} from './ui'

const KINDS: { id: DictKind; label: string }[] = [
  { id: 'categories', label: 'Категории' },
  { id: 'brands', label: 'Бренды' },
  { id: 'statuses', label: 'Статусы' },
]

/** 7 пресетных пар цвет/фон для статусов (design.md §2) */
const COLOR_PRESETS: { color: string; bg: string }[] = [
  { color: '#2E9E5B', bg: '#C8FCD2' },
  { color: '#A87C0F', bg: '#FBFCC8' },
  { color: '#D64545', bg: '#FAD8D1' },
  { color: '#2E8E86', bg: '#D8F2F0' },
  { color: '#5E629B', bg: '#EDEDF7' },
  { color: '#E0235B', bg: '#FBD9E4' },
  { color: '#6B6E9E', bg: '#ECECF3' },
]

function usageCount(
  kind: DictKind,
  id: number,
  items: { categoryId: number | null; brandId: number | null; statusId: number | null }[] | undefined
): number {
  if (!items) return 0
  if (kind === 'categories') return items.filter((i) => i.categoryId === id).length
  if (kind === 'brands') return items.filter((i) => i.brandId === id).length
  return items.filter((i) => i.statusId === id).length
}

/* ─── Модалка переименования ──────────────────────────────────────────────── */

function RenameModal({
  kind,
  entry,
  onClose,
}: {
  kind: DictKind
  entry: DictEntryDto | null
  onClose: () => void
}) {
  const toast = useToast()
  const utils = trpc.useUtils()
  const [name, setName] = useState(entry?.name ?? '')
  const [colorIdx, setColorIdx] = useState(() => {
    if (!entry?.color) return 0
    const i = COLOR_PRESETS.findIndex((p) => p.color === entry.color)
    return i >= 0 ? i : 0
  })

  const update = trpc.admin.dictionaries.update.useMutation({
    onSuccess: () => {
      utils.admin.dictionaries.list.invalidate()
      toast('Запись обновлена')
      onClose()
    },
    onError: (e) => toast(e.message, 'error'),
  })

  return (
    <Modal open={!!entry} onClose={onClose} title="Переименовать">
      {entry && (
        <form
          className="space-y-4"
          onSubmit={(e) => {
            e.preventDefault()
            if (!name.trim()) return
            const preset = kind === 'statuses' ? COLOR_PRESETS[colorIdx] : undefined
            update.mutate({
              kind,
              id: entry.id,
              name: name.trim(),
              ...(preset ? { color: preset.color, bg: preset.bg } : {}),
            })
          }}
        >
          <Field label="Название" required>
            <input className={inputCls} value={name} onChange={(e) => setName(e.target.value)} />
          </Field>
          {kind === 'statuses' && (
            <Field label="Цвет статуса">
              <div className="flex gap-2">
                {COLOR_PRESETS.map((p, i) => (
                  <button
                    key={p.color}
                    type="button"
                    onClick={() => setColorIdx(i)}
                    className={cn(
                      'h-7 w-7 rounded-full border-2 transition',
                      i === colorIdx
                        ? 'border-brand-600 scale-110'
                        : 'border-transparent hover:scale-105'
                    )}
                    style={{ background: p.bg, boxShadow: `inset 0 0 0 6px ${p.bg}` }}
                    aria-label={`Цвет ${i + 1}`}
                  >
                    <span
                      className="mx-auto block h-3 w-3 rounded-full"
                      style={{ background: p.color }}
                    />
                  </button>
                ))}
              </div>
            </Field>
          )}
          <div className="flex justify-end gap-2 pt-1">
            <button type="button" className={btnSecondaryCls} onClick={onClose}>
              Отмена
            </button>
            <button type="submit" className={btnPrimaryCls} disabled={!name.trim() || update.isPending}>
              {update.isPending ? 'Сохранение…' : 'Сохранить'}
            </button>
          </div>
        </form>
      )}
    </Modal>
  )
}

/* ─── Раздел «Справочники» ────────────────────────────────────────────────── */

export default function DictionariesSection() {
  const toast = useToast()
  const utils = trpc.useUtils()
  const [kind, setKind] = useState<DictKind>('categories')
  const [wsFilter, setWsFilter] = useState<string>('')
  const [newName, setNewName] = useState('')
  const [newColorIdx, setNewColorIdx] = useState(0)
  const [renameEntry, setRenameEntry] = useState<DictEntryDto | null>(null)
  const [orderIds, setOrderIds] = useState<number[] | null>(null)

  const { data: workspaces } = trpc.admin.workspaces.list.useQuery()
  const listInput = useMemo(
    () => ({
      kind,
      ...(wsFilter ? { workspaceId: Number(wsFilter) } : {}),
    }),
    [kind, wsFilter]
  )
  const { data: entries, isLoading } = trpc.admin.dictionaries.list.useQuery(listInput)
  const { data: items } = trpc.reports.allItems.useQuery({})

  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      setOrderIds(null)
      setNewName('')
    })
    return () => cancelAnimationFrame(frame)
  }, [kind, wsFilter])

  const ordered = useMemo(() => {
    const rows = entries ?? []
    if (!orderIds) return rows
    const byId = new Map(rows.map((r) => [r.id, r]))
    const sorted = orderIds.map((id) => byId.get(id)).filter(Boolean) as DictEntryDto[]
    const missing = rows.filter((r) => !orderIds.includes(r.id))
    return [...sorted, ...missing]
  }, [entries, orderIds])

  const create = trpc.admin.dictionaries.create.useMutation({
    onSuccess: () => {
      utils.admin.dictionaries.list.invalidate()
      toast('Запись добавлена')
      setNewName('')
    },
    onError: (e) => toast(e.message, 'error'),
  })
  const remove = trpc.admin.dictionaries.remove.useMutation({
    onSuccess: () => {
      utils.admin.dictionaries.list.invalidate()
      toast('Запись удалена')
    },
    onError: (e) => toast(e.message, 'error'),
  })

  const kindLabel = KINDS.find((k) => k.id === kind)?.label ?? ''

  return (
    <section>
      <SectionHeader title="Справочники" />

      <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div className="inline-flex rounded-xl border border-brand-100 bg-surface p-1">
          {KINDS.map((k) => (
            <button
              key={k.id}
              type="button"
              onClick={() => setKind(k.id)}
              className={cn(
                'rounded-lg px-4 py-2 text-sm font-semibold transition',
                kind === k.id ? 'bg-brand-50 text-brand-600' : 'text-ink-500 hover:text-ink-900'
              )}
            >
              {k.label}
            </button>
          ))}
        </div>
        <select
          className={cn(inputCls, 'h-10 w-auto appearance-none pr-8 text-sm')}
          value={wsFilter}
          onChange={(e) => setWsFilter(e.target.value)}
          title="Фильтр по рабочему пространству"
        >
          <option value="">Текущее пространство</option>
          {(workspaces ?? []).map((w) => (
            <option key={w.id} value={w.id}>
              {w.name}
            </option>
          ))}
        </select>
      </div>

      <div className={cn(cardCls, 'overflow-hidden')}>
        {isLoading && <div className="p-6 text-sm text-ink-500">Загрузка…</div>}
        {!isLoading && ordered.length === 0 && (
          <div className="p-6 text-sm text-ink-500">
            Записей пока нет — добавьте первую в поле ниже.
          </div>
        )}
        {!isLoading && ordered.length > 0 && (
          <Reorder.Group
            axis="y"
            values={ordered}
            onReorder={(v) => setOrderIds(v.map((r) => r.id))}
            className="divide-y divide-brand-100/70"
          >
            <TooltipProvider delayDuration={150}>
              {ordered.map((e) => {
                const used = usageCount(kind, e.id, items)
                return (
                  <Reorder.Item
                    key={e.id}
                    value={e}
                    className="flex items-center gap-3 bg-surface px-4 py-3"
                  >
                    <span className="cursor-grab text-ink-300 transition hover:text-brand-600 active:cursor-grabbing">
                      <GripVertical size={16} />
                    </span>
                    {kind === 'statuses' ? (
                      <span
                        className="h-3 w-3 shrink-0 rounded-full"
                        style={{ background: e.color ?? '#9B9EC4' }}
                      />
                    ) : (
                      <Tag size={15} className="shrink-0 text-ink-300" />
                    )}
                    <span className="min-w-0 flex-1 truncate text-sm font-semibold text-ink-900">
                      {e.name}
                    </span>
                    <span className="font-mono-num shrink-0 text-ink-500">{used}</span>
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <button
                          type="button"
                          className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-xl text-ink-500 transition hover:bg-brand-100/60 hover:text-ink-900"
                          aria-label="Действия"
                        >
                          <MoreHorizontal size={18} />
                        </button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="w-52">
                        <DropdownMenuItem onClick={() => setRenameEntry(e)}>
                          <Pencil size={16} className="mr-2" />
                          Переименовать
                        </DropdownMenuItem>
                        {used > 0 ? (
                          <Tooltip>
                            <TooltipTrigger asChild>
                              <DropdownMenuItem
                                disabled
                                className="disabled:opacity-50"
                                onSelect={(ev) => ev.preventDefault()}
                              >
                                <Trash2 size={16} className="mr-2" />
                                Удалить
                              </DropdownMenuItem>
                            </TooltipTrigger>
                            <TooltipContent side="left">
                              Используется в {used} {plural(used, ['карточке', 'карточках', 'карточках'])}
                            </TooltipContent>
                          </Tooltip>
                        ) : (
                          <DropdownMenuItem
                            className="text-danger focus:text-danger"
                            onClick={() => remove.mutate({ kind, id: e.id })}
                          >
                            <Trash2 size={16} className="mr-2" />
                            Удалить
                          </DropdownMenuItem>
                        )}
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </Reorder.Item>
                )
              })}
            </TooltipProvider>
          </Reorder.Group>
        )}

        {/* Строка добавления */}
        <form
          className="flex flex-wrap items-center gap-2 border-t border-brand-100/70 bg-brand-50/40 p-4"
          onSubmit={(e) => {
            e.preventDefault()
            if (!newName.trim()) return
            const preset = kind === 'statuses' ? COLOR_PRESETS[newColorIdx] : undefined
            create.mutate({
              kind,
              name: newName.trim(),
              ...(wsFilter ? { workspaceId: Number(wsFilter) } : {}),
              ...(preset ? { color: preset.color, bg: preset.bg } : {}),
            })
          }}
        >
          <input
            className={cn(inputCls, 'h-10 flex-1 min-w-[180px] text-sm')}
            placeholder={`Новая запись в «${kindLabel}»`}
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
          />
          {kind === 'statuses' && (
            <div className="flex items-center gap-1.5">
              {COLOR_PRESETS.map((p, i) => (
                <button
                  key={p.color}
                  type="button"
                  onClick={() => setNewColorIdx(i)}
                  className={cn(
                    'h-6 w-6 rounded-full border-2 transition',
                    i === newColorIdx ? 'border-brand-600 scale-110' : 'border-transparent'
                  )}
                  style={{ background: p.color }}
                  aria-label={`Цвет ${i + 1}`}
                />
              ))}
            </div>
          )}
          <button
            type="submit"
            className={cn(btnSecondaryCls, 'h-10')}
            disabled={!newName.trim() || create.isPending}
          >
            <Plus size={16} />
            Добавить
          </button>
        </form>
      </div>

      <RenameModal key={renameEntry?.id ?? 'none'} kind={kind} entry={renameEntry} onClose={() => setRenameEntry(null)} />
    </section>
  )
}
