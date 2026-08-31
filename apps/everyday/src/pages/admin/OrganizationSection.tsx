import { useMemo, useState } from 'react'
import { ChevronRight, Network, Plus, Trash2 } from 'lucide-react'
import { trpc } from '@/providers/trpc'
import { btnPrimaryCls, btnSecondaryCls, cardCls, inputCls, SectionHeader, useToast } from './ui'

type Node = {
  id: number
  parentId: number | null
  kind: string
  name: string
  tabLabel: string | null
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
  const create = trpc.admin.organizationNodes.create.useMutation({
    onSuccess: () => {
      utils.admin.organizationNodes.list.invalidate()
      setName('')
      setTabLabel('')
      toast('Раздел добавлен в подписанную историю')
    },
    onError: (error) => toast(error.message, 'error'),
  })
  const remove = trpc.admin.organizationNodes.remove.useMutation({
    onSuccess: () => utils.admin.organizationNodes.list.invalidate(),
    onError: (error) => toast(error.message, 'error'),
  })

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
          create.mutate({
            name: name.trim(),
            kind,
            parentId: parentId ? Number(parentId) : null,
            tabLabel: tabLabel.trim() || null,
          })
        }}
      >
        <input className={inputCls} aria-label="Название раздела" placeholder="Например, кабинет 204" value={name} onChange={(e) => setName(e.target.value)} />
        <select className={inputCls} aria-label="Тип раздела" value={kind} onChange={(e) => setKind(e.target.value)}>
          {KINDS.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
        </select>
        <select className={inputCls} aria-label="Родительский раздел" value={parentId} onChange={(e) => setParentId(e.target.value)}>
          <option value="">Верхний уровень</option>
          {rows.map((node) => <option key={node.id} value={node.id}>{'— '.repeat(node.depth)}{node.name}</option>)}
        </select>
        <input className={inputCls} aria-label="Название вкладки" placeholder="Название вкладки (необязательно)" value={tabLabel} onChange={(e) => setTabLabel(e.target.value)} />
        <button className={btnPrimaryCls} type="submit" disabled={!name.trim() || create.isPending}>
          <Plus size={17} /> Добавить
        </button>
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
                <button
                  type="button"
                  className={btnSecondaryCls}
                  aria-label={'Архивировать ' + node.name}
                  onClick={() => remove.mutate({ id: node.id })}
                >
                  <Trash2 size={16} />
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  )
}
