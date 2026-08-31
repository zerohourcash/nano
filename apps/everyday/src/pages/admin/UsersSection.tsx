import { useMemo, useState } from 'react'
import { motion } from 'framer-motion'
import {
  Info,
  MoreHorizontal,
  Search,
  ShieldCheck,
  Trash2,
  UserPlus,
  UserX,
  UserCheck,
} from 'lucide-react'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import type { RoleRights } from '@db/schema'
import { trpc } from '@/providers/trpc'
import { cn } from '@/lib/utils'
import type { AdminUser } from './types'
import {
  Modal,
  ConfirmModal,
  Field,
  SectionHeader,
  TablePlaceholder,
  Toggle,
  UserAvatar,
  btnPrimaryCls,
  btnSecondaryCls,
  cardCls,
  fmtDate,
  formatPhoneInput,
  inputCls,
  tdCls,
  thCls,
  useToast,
} from './ui'

/* ─── 16 гранулярных прав (ключи RoleRights из API) ───────────────────────── */

const RIGHTS_META: { key: keyof RoleRights; label: string; hint: string }[] = [
  { key: 'viewItems', label: 'Просмотр каталога', hint: 'Открывать список и карточки инструментов' },
  { key: 'createItems', label: 'Создавать карточки инструментов', hint: 'Добавление новых инструментов и материалов' },
  { key: 'editItems', label: 'Редактировать карточки', hint: 'Изменение данных, фото и документов' },
  { key: 'deleteItems', label: 'Удалять карточки', hint: 'Безвозвратное удаление карточек из каталога' },
  { key: 'transferItems', label: 'Передавать инструменты', hint: 'Создание передач от своего имени' },
  { key: 'acceptTransfers', label: 'Принимать передачи', hint: 'Подтверждение приёма или отказ с фото' },
  { key: 'writeOff', label: 'Списание материалов', hint: 'Уменьшение остатков и списание инструментов' },
  { key: 'replenish', label: 'Пополнение материалов', hint: 'Приход количественных материалов на склад' },
  { key: 'inventory', label: 'Проведение инвентаризации', hint: 'Создание сессий сверки и отметка результатов' },
  { key: 'viewHistory', label: 'Просмотр истории', hint: 'Журнал операций, перемещения и списания' },
  { key: 'viewReports', label: 'Просмотр отчётов', hint: 'Отчёты по сотрудникам и полный реестр' },
  { key: 'manageUsers', label: 'Управление пользователями', hint: 'Приглашение, блокировка и права доступа' },
  { key: 'manageWorkspaces', label: 'Управление рабочим пространством', hint: 'Название, часовой пояс, шаблон вн. номеров' },
  { key: 'manageStorages', label: 'Управление складами', hint: 'Создание, редактирование и архивация складов' },
  { key: 'manageSites', label: 'Управление объектами', hint: 'Создание и закрытие строительных объектов' },
  { key: 'manageDictionaries', label: 'Управление справочниками', hint: 'Категории, бренды и статусы' },
]

function allRights(v: boolean): RoleRights {
  return RIGHTS_META.reduce((acc, r) => ({ ...acc, [r.key]: v }), {} as RoleRights)
}

const FALLBACK_DEFAULT_RIGHTS: RoleRights = {
  viewItems: true,
  createItems: true,
  editItems: true,
  deleteItems: false,
  transferItems: true,
  acceptTransfers: true,
  writeOff: false,
  replenish: true,
  inventory: true,
  viewHistory: true,
  viewReports: true,
  manageUsers: false,
  manageWorkspaces: false,
  manageStorages: false,
  manageSites: false,
  manageDictionaries: false,
}

function isOwner(rights: RoleRights | null | undefined): boolean {
  if (!rights) return false
  return RIGHTS_META.every((r) => rights[r.key])
}

const STATUS_META: Record<string, { label: string; cls: string }> = {
  active: { label: 'Активен', cls: 'bg-success-bg text-success' },
  invited: { label: 'Приглашён', cls: 'bg-warning-bg text-warning' },
  disabled: { label: 'Заблокирован', cls: 'bg-danger-bg text-danger' },
}

/* ─── Модалка приглашения ─────────────────────────────────────────────────── */

function InviteModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const toast = useToast()
  const utils = trpc.useUtils()
  const [fullName, setFullName] = useState('')
  const [phone, setPhone] = useState('')
  const [position, setPosition] = useState('')

  const invite = trpc.admin.users.invite.useMutation({
    onSuccess: () => {
      utils.admin.users.list.invalidate()
      toast('Сотрудник добавлен. Можно войти по этому телефону.')
      setFullName('')
      setPhone('')
      setPosition('')
      onClose()
    },
    onError: (e) => toast(e.message, 'error'),
  })

  const valid = fullName.trim().length > 1 && phone.replace(/\D/g, '').length === 11

  return (
    <Modal open={open} onClose={onClose} title="Пригласить пользователя">
      <form
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault()
          if (!valid) return
          invite.mutate({
            fullName: fullName.trim(),
            phone,
            position: position.trim() || undefined,
          })
        }}
      >
        <Field label="Телефон" required>
          <input
            className={cn(inputCls, 'font-mono-num')}
            placeholder="+7 921 555-00-00"
            inputMode="tel"
            value={phone}
            onChange={(e) => setPhone(formatPhoneInput(e.target.value))}
          />
        </Field>
        <Field label="ФИО" required>
          <input
            className={inputCls}
            placeholder="Иван Петров"
            value={fullName}
            onChange={(e) => setFullName(e.target.value)}
          />
        </Field>
        <Field label="Должность">
          <input
            className={inputCls}
            placeholder="Монтажник"
            value={position}
            onChange={(e) => setPosition(e.target.value)}
          />
        </Field>
        <div className="rounded-xl border-l-[3px] border-teal bg-info-bg px-4 py-3 text-sm leading-5 text-ink-900">
          Сотрудник активирует аккаунт по персональному приглашению и задаст пароль не короче 12 символов.
        </div>
        <div className="flex justify-end gap-2 pt-1">
          <button type="button" className={btnSecondaryCls} onClick={onClose}>
            Отмена
          </button>
          <button type="submit" className={btnPrimaryCls} disabled={!valid || invite.isPending}>
            {invite.isPending ? 'Отправка…' : 'Отправить приглашение'}
          </button>
        </div>
      </form>
    </Modal>
  )
}

/* ─── Модалка прав доступа (720px, 16 тогглов, пресеты) ───────────────────── */

function RightsModal({
  user,
  onClose,
}: {
  user: AdminUser | null
  onClose: () => void
}) {
  const toast = useToast()
  const utils = trpc.useUtils()
  const { data: defaultRights } = trpc.admin.users.defaultRights.useQuery()
  const [rights, setRights] = useState<RoleRights | null>(null)
  const [requireApproval, setRequireApproval] = useState<boolean | null>(null)
  const [allowNoDue, setAllowNoDue] = useState<boolean | null>(null)
  const [maxHours, setMaxHours] = useState<string | null>(null)

  const current: RoleRights =
    rights ?? user?.roleRights ?? defaultRights ?? FALLBACK_DEFAULT_RIGHTS
  const policy = user && 'checkoutPolicy' in (user as object) ? (user as { checkoutPolicy?: { requireApproval?: boolean; allowNoDueDate?: boolean; maxHours?: number | null } }).checkoutPolicy : undefined
  const reqAppr = requireApproval ?? policy?.requireApproval ?? false
  const noDue = allowNoDue ?? policy?.allowNoDueDate ?? true
  const hours = maxHours ?? (policy?.maxHours != null ? String(policy.maxHours) : '')

  const update = trpc.admin.users.update.useMutation({
    onSuccess: () => {
      utils.admin.users.list.invalidate()
      toast('Права доступа обновлены')
      setRights(null)
      onClose()
    },
    onError: (e) => toast(e.message, 'error'),
  })

  const applyPreset = (name: string) => {
    if (name === 'Владелец') setRights(allRights(true))
    else if (name === 'Кладовщик') setRights({ ...(defaultRights ?? FALLBACK_DEFAULT_RIGHTS) })
    else if (name === 'Прораб')
      setRights({
        ...allRights(false),
        viewItems: true,
        transferItems: true,
        acceptTransfers: true,
        replenish: true,
        inventory: true,
        viewHistory: true,
        viewReports: true,
      })
    else
      setRights({
        ...allRights(false),
        viewItems: true,
        viewHistory: true,
        viewReports: true,
      })
  }

  return (
    <Modal
      open={!!user}
      onClose={() => {
        setRights(null)
        onClose()
      }}
      title="Права доступа"
      maxWidth={720}
    >
      {user && (
        <div key={user.id}>
          <div className="mb-4 flex items-center gap-3">
            <UserAvatar name={user.fullName} url={user.avatarUrl} size={44} />
            <div className="min-w-0">
              <div className="truncate text-[15px] leading-[22px] font-semibold text-ink-900">
                {user.fullName}
              </div>
              <div className="text-[13px] leading-[18px] text-ink-500">
                {user.position ?? '—'} · <span className="font-mono-num">{user.phone}</span>
              </div>
            </div>
          </div>

          <div className="mb-4 flex flex-wrap gap-2">
            {['Владелец', 'Кладовщик', 'Прораб', 'Наблюдатель'].map((p) => (
              <button
                key={p}
                type="button"
                onClick={() => applyPreset(p)}
                className="rounded-full border border-brand-100 bg-surface px-3.5 py-1.5 text-[13px] font-semibold text-brand-600 transition hover:bg-brand-50 active:scale-[0.97]"
              >
                {p}
              </button>
            ))}
          </div>

          <TooltipProvider delayDuration={150}>
            <div className="grid grid-cols-1 gap-x-6 md:grid-cols-2">
              {RIGHTS_META.map((r, i) => (
                <motion.div
                  key={r.key}
                  initial={{ opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ duration: 0.2, delay: i * 0.02 }}
                  className="flex items-start gap-3 border-b border-brand-100/60 py-3"
                >
                  <div className="pt-0.5">
                    <Toggle
                      checked={current[r.key]}
                      onChange={(v) => setRights({ ...current, [r.key]: v })}
                    />
                  </div>
                  <div className="min-w-0">
                    <div className="flex items-center gap-1.5 text-sm leading-5 font-semibold text-ink-900">
                      {r.label}
                      <Tooltip>
                        <TooltipTrigger asChild>
                          <span className="inline-flex cursor-help text-ink-300 transition hover:text-brand-600">
                            <Info size={14} />
                          </span>
                        </TooltipTrigger>
                        <TooltipContent side="top" className="max-w-[240px]">
                          {r.hint}
                        </TooltipContent>
                      </Tooltip>
                    </div>
                    <div className="text-xs leading-4 text-ink-500">{r.hint}</div>
                  </div>
                </motion.div>
              ))}
            </div>
          </TooltipProvider>

          <div className="mt-5 rounded-xl border border-brand-100 p-4 space-y-3">
            <div className="text-sm font-semibold text-ink-900">Правила выдачи</div>
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={reqAppr} onChange={(e) => setRequireApproval(e.target.checked)} />
              Выдача только после одобрения администратора
            </label>
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={noDue} onChange={(e) => setAllowNoDue(e.target.checked)} />
              Разрешить выдачу без срока возврата
            </label>
            <div>
              <div className="text-[13px] text-ink-500 mb-1">Максимальный срок, часов (пусто = без лимита)</div>
              <input
                className={inputCls}
                type="number"
                min={1}
                value={hours}
                onChange={(e) => setMaxHours(e.target.value)}
                placeholder="например 24"
              />
            </div>
          </div>

          <div className="mt-5 flex justify-end gap-2">
            <button
              type="button"
              className={btnSecondaryCls}
              onClick={() => {
                setRights(null)
                onClose()
              }}
            >
              Отмена
            </button>
            <button
              type="button"
              className={btnPrimaryCls}
              disabled={update.isPending}
              onClick={() =>
                update.mutate({
                  id: user.id,
                  roleRights: current,
                  checkoutPolicy: {
                    requireApproval: reqAppr,
                    allowNoDueDate: noDue,
                    maxHours: hours.trim() ? Number(hours) : null,
                    allowedCategoryIds: null,
                  },
                })
              }
            >
              {update.isPending ? 'Сохранение…' : 'Сохранить права'}
            </button>
          </div>
        </div>
      )}
    </Modal>
  )
}

/* ─── Раздел «Пользователи» ───────────────────────────────────────────────── */

export default function UsersSection() {
  const toast = useToast()
  const utils = trpc.useUtils()
  const { data: users, isLoading } = trpc.admin.users.list.useQuery({})
  const { data: workspaces } = trpc.admin.workspaces.list.useQuery()

  const [search, setSearch] = useState('')
  const [inviteOpen, setInviteOpen] = useState(false)
  const [rightsUser, setRightsUser] = useState<AdminUser | null>(null)
  const [removeUser, setRemoveUser] = useState<AdminUser | null>(null)

  const update = trpc.admin.users.update.useMutation({
    onSuccess: (_d, vars) => {
      utils.admin.users.list.invalidate()
      toast(vars.status === 'disabled' ? 'Пользователь заблокирован' : 'Пользователь разблокирован')
    },
    onError: (e) => toast(e.message, 'error'),
  })
  const remove = trpc.admin.users.remove.useMutation({
    onSuccess: (res) => {
      utils.admin.users.list.invalidate()
      toast(
        res?.deleted === false
          ? 'Участник исключён, история его операций сохранена'
          : 'Учётная запись удалена',
      )
      setRemoveUser(null)
    },
    onError: (e) => toast(e.message, 'error'),
  })

  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase()
    if (!q) return users ?? []
    return (users ?? []).filter((u) => {
      const parts = u.fullName.toLowerCase().split(/\s+/)
      const surname = parts[1] ?? parts[0] ?? ''
      return surname.includes(q) || u.fullName.toLowerCase().includes(q)
    })
  }, [users, search])

  const currentWsName = workspaces?.[0]?.name ?? '—'

  return (
    <section>
      <SectionHeader
        title="Пользователи"
        count={users?.length}
        action={
          <button type="button" className={btnPrimaryCls} onClick={() => setInviteOpen(true)}>
            <UserPlus size={16} />
            Пригласить
          </button>
        }
      />

      <div className={cn(cardCls, 'overflow-hidden')}>
        <div className="border-b border-brand-100/70 p-4">
          <div className="relative max-w-sm">
            <Search size={16} className="absolute left-3.5 top-1/2 -translate-y-1/2 text-ink-300" />
            <input
              className={cn(inputCls, 'pl-10')}
              placeholder="Поиск по фамилии"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </div>
        </div>

        <div className="overflow-x-auto">
          <table className="w-full min-w-[760px] border-collapse">
            <thead>
              <tr>
                <th className={thCls}>Пользователь</th>
                <th className={thCls}>Телефон</th>
                <th className={thCls}>Добавлен</th>
                <th className={thCls}>Пространства</th>
                <th className={thCls}>Статус</th>
                <th className={cn(thCls, 'w-12')} />
              </tr>
            </thead>
            <tbody>
              {isLoading && <TablePlaceholder colSpan={6} text="Загрузка…" />}
              {!isLoading && filtered.length === 0 && (
                <TablePlaceholder
                  colSpan={6}
                  text={search ? 'Никого не найдено по такой фамилии' : 'Пользователей пока нет'}
                />
              )}
              {filtered.map((u, i) => {
                const st = STATUS_META[u.status] ?? STATUS_META.active
                return (
                  <motion.tr
                    key={u.id}
                    initial={{ opacity: 0, y: 8 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ duration: 0.2, delay: Math.min(i, 11) * 0.03 }}
                    className="transition-colors hover:bg-brand-50"
                  >
                    <td className={tdCls}>
                      <div className="flex items-center gap-3">
                        <UserAvatar name={u.fullName} url={u.avatarUrl} />
                        <div className="min-w-0">
                          <div className="flex items-center gap-2 text-[15px] leading-5 font-semibold text-ink-900">
                            <span className="truncate">{u.fullName}</span>
                            {isOwner(u.roleRights) && (
                              <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-brand-50 px-2 py-0.5 text-[11px] font-semibold text-brand-600">
                                <ShieldCheck size={12} />
                                Владелец
                              </span>
                            )}
                          </div>
                          <div className="text-[13px] leading-[18px] text-ink-500">
                            {u.position ?? '—'}
                          </div>
                        </div>
                      </div>
                    </td>
                    <td className={cn(tdCls, 'font-mono-num whitespace-nowrap')}>{u.phone}</td>
                    <td className={cn(tdCls, 'font-mono-num text-ink-500 whitespace-nowrap')}>
                      {fmtDate(u.createdAt)}
                    </td>
                    <td className={tdCls}>
                      <span className="inline-flex items-center rounded-full bg-brand-50 px-2.5 py-0.5 text-xs font-semibold text-brand-600">
                        {currentWsName}
                      </span>
                    </td>
                    <td className={tdCls}>
                      <span
                        className={cn(
                          'inline-flex items-center rounded-full px-2.5 py-0.5 text-caption',
                          st.cls
                        )}
                      >
                        {st.label}
                      </span>
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
                        <DropdownMenuContent align="end" className="w-56">
                          <DropdownMenuItem onClick={() => setRightsUser(u)}>
                            <ShieldCheck size={16} className="mr-2" />
                            Права доступа
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onClick={() =>
                              update.mutate({
                                id: u.id,
                                status: u.status === 'disabled' ? 'active' : 'disabled',
                              })
                            }
                          >
                            {u.status === 'disabled' ? (
                              <>
                                <UserCheck size={16} className="mr-2" />
                                Разблокировать
                              </>
                            ) : (
                              <>
                                <UserX size={16} className="mr-2" />
                                Заблокировать
                              </>
                            )}
                          </DropdownMenuItem>
                          <DropdownMenuSeparator />
                          <DropdownMenuItem
                            className="text-danger focus:text-danger"
                            onClick={() => setRemoveUser(u)}
                          >
                            <Trash2 size={16} className="mr-2" />
                            Удалить
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

      <InviteModal open={inviteOpen} onClose={() => setInviteOpen(false)} />
      <RightsModal user={rightsUser} onClose={() => setRightsUser(null)} />
      <ConfirmModal
        open={!!removeUser}
        onClose={() => setRemoveUser(null)}
        onConfirm={() =>
          removeUser && remove.mutate({ id: removeUser.id, workspaceId: workspaces?.[0]?.id })
        }
        title="Исключить участника?"
        text={`${removeUser?.fullName ?? ''} будет удалён из рабочего пространства. История операций сохранится.`}
        loading={remove.isPending}
      />
    </section>
  )
}
