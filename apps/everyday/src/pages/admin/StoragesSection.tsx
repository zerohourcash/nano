import { useMemo, useState } from 'react'
import { Link } from 'react-router'
import { motion } from 'framer-motion'
import { Archive, MoreHorizontal, Pencil, Plus } from 'lucide-react'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { trpc } from '@/providers/trpc'
import { cn } from '@/lib/utils'
import type { StorageDto } from './types'
import {
  ConfirmModal,
  Field,
  Modal,
  SectionHeader,
  TablePlaceholder,
  UserAvatar,
  btnPrimaryCls,
  btnSecondaryCls,
  cardCls,
  inputCls,
  plural,
  tdCls,
  thCls,
  useToast,
} from './ui'

/* ─── Модалка создания / редактирования склада ────────────────────────────── */

function StorageModal({
  storage,
  open,
  onClose,
}: {
  storage: StorageDto | null
  open: boolean
  onClose: () => void
}) {
  const toast = useToast()
  const utils = trpc.useUtils()
  const { data: users } = trpc.admin.users.list.useQuery({})
  const [name, setName] = useState(storage?.name ?? '')
  const [responsibleId, setResponsibleId] = useState<string>(
    storage?.responsibleUserId ? String(storage.responsibleUserId) : ''
  )
  const [address, setAddress] = useState(storage?.address ?? '')

  const isEdit = !!storage
  const invalidate = () => utils.admin.storages.list.invalidate()
  const create = trpc.admin.storages.create.useMutation({
    onSuccess: () => {
      invalidate()
      toast('Склад добавлен')
      onClose()
    },
    onError: (e) => toast(e.message, 'error'),
  })
  const update = trpc.admin.storages.update.useMutation({
    onSuccess: () => {
      invalidate()
      toast('Склад обновлён')
      onClose()
    },
    onError: (e) => toast(e.message, 'error'),
  })
  const pending = create.isPending || update.isPending

  return (
    <Modal open={open} onClose={onClose} title={isEdit ? 'Редактировать склад' : 'Добавить склад'}>
      <form
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault()
          if (!name.trim()) return
          const responsibleUserId = responsibleId ? Number(responsibleId) : null
          if (isEdit && storage)
            update.mutate({
              id: storage.id,
              name: name.trim(),
              responsibleUserId,
              address: address.trim() || null,
            })
          else
            create.mutate({
              name: name.trim(),
              responsibleUserId,
              address: address.trim() || undefined,
            })
        }}
      >
        <Field label="Название" required>
          <input
            className={inputCls}
            placeholder="Склад №3"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </Field>
        <Field label="Ответственный">
          <select
            className={cn(inputCls, 'appearance-none pr-10')}
            value={responsibleId}
            onChange={(e) => setResponsibleId(e.target.value)}
          >
            <option value="">Не назначен</option>
            {(users ?? []).map((u) => (
              <option key={u.id} value={u.id}>
                {u.fullName}
                {u.position ? ` — ${u.position}` : ''}
              </option>
            ))}
          </select>
        </Field>
        <Field label="Адрес">
          <input
            className={inputCls}
            placeholder="СПб, Индустриальный пр. 44, стр. 2"
            value={address}
            onChange={(e) => setAddress(e.target.value)}
          />
        </Field>
        <div className="flex justify-end gap-2 pt-1">
          <button type="button" className={btnSecondaryCls} onClick={onClose}>
            Отмена
          </button>
          <button type="submit" className={btnPrimaryCls} disabled={!name.trim() || pending}>
            {pending ? 'Сохранение…' : isEdit ? 'Сохранить' : 'Добавить'}
          </button>
        </div>
      </form>
    </Modal>
  )
}

/* ─── Раздел «Склады» ─────────────────────────────────────────────────────── */

export default function StoragesSection() {
  const toast = useToast()
  const utils = trpc.useUtils()
  const { data: storages, isLoading } = trpc.admin.storages.list.useQuery({})
  const { data: items } = trpc.reports.allItems.useQuery({})

  const [modalOpen, setModalOpen] = useState(false)
  const [editStorage, setEditStorage] = useState<StorageDto | null>(null)
  const [removeStorage, setRemoveStorage] = useState<StorageDto | null>(null)

  const countByStorage = useMemo(() => {
    const map = new Map<number, number>()
    for (const it of items ?? []) {
      if (it.storageId != null) map.set(it.storageId, (map.get(it.storageId) ?? 0) + 1)
    }
    return map
  }, [items])

  const remove = trpc.admin.storages.remove.useMutation({
    onSuccess: () => {
      utils.admin.storages.list.invalidate()
      toast('Склад архивирован')
      setRemoveStorage(null)
    },
    onError: (e) => toast(e.message, 'error'),
  })

  return (
    <section>
      <SectionHeader
        title="Склады"
        count={storages?.length}
        action={
          <button
            type="button"
            className={btnPrimaryCls}
            onClick={() => {
              setEditStorage(null)
              setModalOpen(true)
            }}
          >
            <Plus size={16} />
            Добавить склад
          </button>
        }
      />

      <div className={cn(cardCls, 'overflow-hidden')}>
        <div className="overflow-x-auto">
          <table className="w-full min-w-[720px] border-collapse">
            <thead>
              <tr>
                <th className={thCls}>Название</th>
                <th className={thCls}>Ответственный</th>
                <th className={thCls}>Адрес</th>
                <th className={thCls}>Единиц</th>
                <th className={cn(thCls, 'w-12')} />
              </tr>
            </thead>
            <tbody>
              {isLoading && <TablePlaceholder colSpan={5} text="Загрузка…" />}
              {!isLoading && (storages ?? []).length === 0 && (
                <TablePlaceholder colSpan={5} text="Складов пока нет — добавьте первый" />
              )}
              {(storages ?? []).map((s, i) => {
                const count = countByStorage.get(s.id) ?? 0
                return (
                  <motion.tr
                    key={s.id}
                    initial={{ opacity: 0, y: 8 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ duration: 0.2, delay: Math.min(i, 11) * 0.03 }}
                    className="transition-colors hover:bg-brand-50"
                  >
                    <td className={cn(tdCls, 'font-semibold text-ink-900')}>{s.name}</td>
                    <td className={tdCls}>
                      {s.responsible ? (
                        <span className="flex items-center gap-2">
                          <UserAvatar
                            name={s.responsible.fullName}
                            url={s.responsible.avatarUrl}
                            size={24}
                          />
                          <span className="text-sm text-ink-900">{s.responsible.fullName}</span>
                        </span>
                      ) : (
                        <span className="text-sm text-ink-300">Не назначен</span>
                      )}
                    </td>
                    <td className={cn(tdCls, 'text-ink-500')}>{s.address ?? '—'}</td>
                    <td className={tdCls}>
                      <Link
                        to={`/?storageId=${s.id}`}
                        className="font-mono-num text-brand-600 transition hover:underline"
                        title={`Показать ${count} ${plural(count, ['единица', 'единицы', 'единиц'])} в каталоге`}
                      >
                        {count}
                      </Link>
                    </td>
                    <td className={cn(tdCls, 'text-right')}>
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <button
                            type="button"
                            className="inline-flex h-8 w-8 items-center justify-center rounded-xl text-ink-500 transition hover:bg-brand-100/60 hover:text-ink-900"
                            aria-label="Действия"
                          >
                            <MoreHorizontal size={18} />
                          </button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end" className="w-52">
                          <DropdownMenuItem
                            onClick={() => {
                              setEditStorage(s)
                              setModalOpen(true)
                            }}
                          >
                            <Pencil size={16} className="mr-2" />
                            Редактировать
                          </DropdownMenuItem>
                          <DropdownMenuSeparator />
                          <DropdownMenuItem
                            className="text-danger focus:text-danger"
                            onClick={() => setRemoveStorage(s)}
                          >
                            <Archive size={16} className="mr-2" />
                            Архивировать
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </td>
                  </motion.tr>
                )
              })}
            </tbody>
          </table>
        </div>
      </div>

      {modalOpen && (
        <StorageModal
          key={editStorage?.id ?? 'new'}
          storage={editStorage}
          open={modalOpen}
          onClose={() => setModalOpen(false)}
        />
      )}
      <ConfirmModal
        open={!!removeStorage}
        onClose={() => setRemoveStorage(null)}
        onConfirm={() => removeStorage && remove.mutate({ id: removeStorage.id })}
        title="Архивировать склад?"
        text={`«${removeStorage?.name ?? ''}» будет скрыт из списков. Привязанные предметы останутся в учёте.`}
        confirmLabel="Архивировать"
        loading={remove.isPending}
      />
    </section>
  )
}
