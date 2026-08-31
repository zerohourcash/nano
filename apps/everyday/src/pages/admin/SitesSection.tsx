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
import type { SiteDto } from './types'
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
  tdCls,
  thCls,
  useToast,
} from './ui'

/* ─── Модалка создания / редактирования объекта ───────────────────────────── */

function SiteModal({
  site,
  open,
  onClose,
}: {
  site: SiteDto | null
  open: boolean
  onClose: () => void
}) {
  const toast = useToast()
  const utils = trpc.useUtils()
  const { data: users } = trpc.admin.users.list.useQuery({})
  const [name, setName] = useState(site?.name ?? '')
  const [responsibleId, setResponsibleId] = useState<string>(
    site?.responsibleUserId ? String(site.responsibleUserId) : ''
  )

  const isEdit = !!site
  const invalidate = () => utils.admin.buildingSites.list.invalidate()
  const create = trpc.admin.buildingSites.create.useMutation({
    onSuccess: () => {
      invalidate()
      toast('Объект добавлен')
      onClose()
    },
    onError: (e) => toast(e.message, 'error'),
  })
  const update = trpc.admin.buildingSites.update.useMutation({
    onSuccess: () => {
      invalidate()
      toast('Объект обновлён')
      onClose()
    },
    onError: (e) => toast(e.message, 'error'),
  })
  const pending = create.isPending || update.isPending

  return (
    <Modal open={open} onClose={onClose} title={isEdit ? 'Редактировать объект' : 'Добавить объект'}>
      <form
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault()
          if (!name.trim()) return
          const responsibleUserId = responsibleId ? Number(responsibleId) : null
          if (isEdit && site)
            update.mutate({ id: site.id, name: name.trim(), responsibleUserId })
          else create.mutate({ name: name.trim(), responsibleUserId })
        }}
      >
        <Field label="Название" required>
          <input
            className={inputCls}
            placeholder="ЖК «Южный парк», корпус 2"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </Field>
        <Field label="Прораб">
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

/* ─── Раздел «Объекты» ────────────────────────────────────────────────────── */

export default function SitesSection() {
  const toast = useToast()
  const utils = trpc.useUtils()
  const { data: sites, isLoading } = trpc.admin.buildingSites.list.useQuery({})
  const { data: items } = trpc.reports.allItems.useQuery({})

  const [modalOpen, setModalOpen] = useState(false)
  const [editSite, setEditSite] = useState<SiteDto | null>(null)
  const [removeSite, setRemoveSite] = useState<SiteDto | null>(null)

  const countBySite = useMemo(() => {
    const map = new Map<number, number>()
    for (const it of items ?? []) {
      if (it.buildingSiteId != null)
        map.set(it.buildingSiteId, (map.get(it.buildingSiteId) ?? 0) + 1)
    }
    return map
  }, [items])

  const remove = trpc.admin.buildingSites.remove.useMutation({
    onSuccess: () => {
      utils.admin.buildingSites.list.invalidate()
      toast('Объект архивирован')
      setRemoveSite(null)
    },
    onError: (e) => toast(e.message, 'error'),
  })

  return (
    <section>
      <SectionHeader
        title="Объекты"
        count={sites?.length}
        action={
          <button
            type="button"
            className={btnPrimaryCls}
            onClick={() => {
              setEditSite(null)
              setModalOpen(true)
            }}
          >
            <Plus size={16} />
            Добавить объект
          </button>
        }
      />

      <div className={cn(cardCls, 'overflow-hidden')}>
        <div className="overflow-x-auto">
          <table className="w-full min-w-[680px] border-collapse">
            <thead>
              <tr>
                <th className={thCls}>Название</th>
                <th className={thCls}>Прораб</th>
                <th className={thCls}>Единиц</th>
                <th className={cn(thCls, 'w-12')} />
              </tr>
            </thead>
            <tbody>
              {isLoading && <TablePlaceholder colSpan={4} text="Загрузка…" />}
              {!isLoading && (sites ?? []).length === 0 && (
                <TablePlaceholder colSpan={4} text="Объектов пока нет — добавьте первый" />
              )}
              {(sites ?? []).map((s, i) => {
                const count = countBySite.get(s.id) ?? 0
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
                    <td className={tdCls}>
                      <Link
                        to={`/?buildingSiteId=${s.id}`}
                        className="font-mono-num text-brand-600 transition hover:underline"
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
                              setEditSite(s)
                              setModalOpen(true)
                            }}
                          >
                            <Pencil size={16} className="mr-2" />
                            Редактировать
                          </DropdownMenuItem>
                          <DropdownMenuSeparator />
                          <DropdownMenuItem
                            className="text-danger focus:text-danger"
                            onClick={() => setRemoveSite(s)}
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
        <SiteModal
          key={editSite?.id ?? 'new'}
          site={editSite}
          open={modalOpen}
          onClose={() => setModalOpen(false)}
        />
      )}
      <ConfirmModal
        open={!!removeSite}
        onClose={() => setRemoveSite(null)}
        onConfirm={() => removeSite && remove.mutate({ id: removeSite.id })}
        title="Архивировать объект?"
        text={`«${removeSite?.name ?? ''}» будет скрыт из списков. Привязанные предметы останутся в учёте.`}
        confirmLabel="Архивировать"
        loading={remove.isPending}
      />
    </section>
  )
}
