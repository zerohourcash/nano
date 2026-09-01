import { useMemo, useState } from 'react'
import { Check, ChevronRight, Network, Pencil, Plus, Trash2, X } from 'lucide-react'
import { trpc } from '@/providers/trpc'
import { btnPrimaryCls, btnSecondaryCls, cardCls, inputCls, SectionHeader, useToast } from './ui'

type Node = {
  id: number
  parentId: number | null
  kind: string
  name: string
  tabLabel: string | null
  displayOrder: number
}

const KINDS = [
  ['division', 'Подразделение'],
  ['site', 'Объект'],
  ['warehouse', 'Склад'],
  ['floor', 'Этаж'],
  ['room', 'Кабинет'],
  ['section', 'Произвольный раздел'],
] as const

function flatten(nodes: Node[], parentId: number | null = null, depth = 0): Array<Node & { depth: number }> {
  return nodes
    .filter((node) => node.parentId === parentId)
    .flatMap((node) => [{ ...node, depth }, ...flatten(nodes, node.id, depth + 1)])
}

function descendantsOf(nodes: Node[], rootId: number): Set<number> {
  const result = new Set<number>([rootId])
  let changed = true
  while (changed) {
    changed = false
    for (const node of nodes) {
      if (node.parentId && result.has(node.parentId) && !result.has(node.id)) {
        result.add(node.id)
        changed = true
      }
    }
  }
  return result
}

export default function OrganizationSection() {
  const toast = useToast()
  const utils = trpc.useUtils()
  const { data = [], isLoading } = trpc.admin.organizationNodes.list.useQuery({})
  const nodes = data as Node[]
  const rows = useMemo(() => flatten(nodes), [nodes])
  const [name, setName] = useState('')
  const [kind, setKind] = useState('division')
  const [parentId, setParentId] = useState('')
  const [tabLabel, setTabLabel] = useState('')
  const [editingId, setEditingId] = useState<number | null>(null)
  const resetForm = () => {
    setName('')
    setKind('division')
    setParentId('')
    setTabLabel('')
    setEditingId(null)
  }
  const create = trpc.admin.organizationNodes.create.useMutation({
    onSuccess: () => {
      utils.admin.organizationNodes.list.invalidate()
      resetForm()
      toast('Раздел добавлен в подписанную историю')
    },
    onError: (error) => toast(error.message, 'error'),
  })
  const update = trpc.admin.organizationNodes.update.useMutation({
    onSuccess: () => {
      utils.admin.organizationNodes.list.invalidate()
      resetForm()
      toast('Изменения раздела подписаны и сохранены')
    },
    onError: (error) => toast(error.message, 'error'),
  })
  const remove = trpc.admin.organizationNodes.remove.useMutation({
    onSuccess: () => utils.admin.organizationNodes.list.invalidate(),
    onError: (error) => toast(error.message, 'error'),
  })
  const forbiddenParents = editingId === null ? new Set<number>() : descendantsOf(nodes, editingId)

  const edit = (node: Node) => {
    setEditingId(node.id)
    setName(node.name)
    setKind(node.kind)
    setParentId(node.parentId ? String(node.parentId) : '')
    setTabLabel(node.tabLabel ?? '')
    window.scrollTo({ top: 0, behavior: 'smooth' })
  }

  return (
    <section className="space-y-4">
      <div>
        <SectionHeader title="Структура организации" />
        <p className="mt-1 text-sm text-ink-500">
          Создайте любое число уровней: подразделения, объекты, склады, этажи,
          кабинеты или собственные типы.
        </p>
      </div>
      <form
        className={cardCls + ' grid gap-3 p-4 md:grid-cols-2 xl:grid-cols-5'}
        onSubmit={(event) => {
          event.preventDefault()
          if (!name.trim()) return
          const fields = {
            name: name.trim(),
            kind,
            parentId: parentId ? Number(parentId) : null,
            tabLabel: tabLabel.trim() || null,
          }
          if (editingId === null) create.mutate(fields)
          else update.mutate({ id: editingId, ...fields })
        }}
      >
        <input className={inputCls} aria-label="Название раздела" placeholder="Например, кабинет 204" value={name} onChange={(e) => setName(e.target.value)} />
        <select className={inputCls} aria-label="Тип раздела" value={kind} onChange={(e) => setKind(e.target.value)}>
          {KINDS.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
        </select>
        <select className={inputCls} aria-label="Родительский раздел" value={parentId} onChange={(e) => setParentId(e.target.value)}>
          <option value="">Верхний уровень</option>
          {rows.filter((node) => !forbiddenParents.has(node.id)).map((node) => <option key={node.id} value={node.id}>{'— '.repeat(node.depth)}{node.name}</option>)}
        </select>
        <input className={inputCls} aria-label="Название вкладки" placeholder="Название вкладки (необязательно)" value={tabLabel} onChange={(e) => setTabLabel(e.target.value)} />
        <div className="flex gap-2">
          <button className={`${btnPrimaryCls} flex-1`} type="submit" disabled={!name.trim() || create.isPending || update.isPending}>
            {editingId === null ? <><Plus size={17} /> Добавить</> : <><Check size={17} /> Сохранить</>}
          </button>
          {editingId !== null && (
            <button className={btnSecondaryCls} type="button" aria-label="Отменить редактирование" onClick={resetForm}>
              <X size={17} />
            </button>
          )}
        </div>
      </form>

      <div className={cardCls}>
        {isLoading ? (
          <p className="p-6 text-sm text-ink-500">Загрузка структуры…</p>
        ) : rows.length === 0 ? (
          <div className="flex flex-col items-center gap-2 p-10 text-center text-ink-500">
            <Network size={30} />
            <p>Структура пока пуста. Добавьте первый раздел.</p>
          </div>
        ) : (
          <ul className="divide-y divide-ink-100">
            {rows.map((node) => (
              <li key={node.id} className="flex items-center gap-3 px-4 py-3">
                <span style={{ width: node.depth * 24 }} className="shrink-0" />
                {node.depth > 0 && <ChevronRight size={15} className="text-ink-300" />}
                <div className="min-w-0 flex-1">
                  <p className="truncate font-semibold text-ink-900">{node.name}</p>
                  <p className="text-xs text-ink-500">{node.kind}{node.tabLabel ? ' · вкладка «' + node.tabLabel + '»' : ''}</p>
                </div>
                <div className="flex gap-2">
                  <button
                    type="button"
                    className={btnSecondaryCls}
                    aria-label={'Изменить ' + node.name}
                    onClick={() => edit(node)}
                  >
                    <Pencil size={16} />
                  </button>
                  <button
                    type="button"
                    className={btnSecondaryCls}
                    aria-label={'Архивировать ' + node.name}
                    onClick={() => remove.mutate({ id: node.id })}
                  >
                    <Trash2 size={16} />
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  )
}
