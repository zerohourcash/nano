import { useCallback, useEffect, useMemo, useState } from 'react'
import { motion } from 'framer-motion'
import { Layers, Plus, QrCode } from 'lucide-react'
import { trpc } from '@/providers/trpc'
import { useStore } from '@/lib/store'
import { joinInviteUrl } from '@/lib/app-mode'
import InviteQrBlock from '@/components/InviteQrBlock'
import { INVITE_ROLES, INVITE_TTL_HOURS, firstUsableInvite, inviteExpiryLabel } from '@/lib/invite'
import type { InviteRole } from '@/lib/invite'
import { cn } from '@/lib/utils'
import type { WorkspaceDto } from './types'
import {
  Field,
  Modal,
  SectionHeader,
  Toggle,
  btnPrimaryCls,
  btnSecondaryCls,
  cardCls,
  fmtDate,
  inputCls,
  plural,
  useToast,
} from './ui'

const TIMEZONES: { value: string; label: string }[] = [
  { value: 'Europe/Kaliningrad', label: 'Калининград, UTC+2' },
  { value: 'Europe/Moscow', label: 'Москва, UTC+3' },
  { value: 'Europe/Samara', label: 'Самара, UTC+4' },
  { value: 'Asia/Yekaterinburg', label: 'Екатеринбург, UTC+5' },
  { value: 'Asia/Omsk', label: 'Омск, UTC+6' },
  { value: 'Asia/Krasnoyarsk', label: 'Красноярск, UTC+7' },
  { value: 'Asia/Irkutsk', label: 'Иркутск, UTC+8' },
  { value: 'Asia/Yakutsk', label: 'Якутск, UTC+9' },
  { value: 'Asia/Vladivostok', label: 'Владивосток, UTC+10' },
  { value: 'Asia/Magadan', label: 'Магадан, UTC+11' },
  { value: 'Asia/Kamchatka', label: 'Камчатка, UTC+12' },
]

function tzLabel(v: string): string {
  return TIMEZONES.find((t) => t.value === v)?.label ?? v
}

/* ─── Форма настроек текущего пространства ────────────────────────────────── */

function WorkspaceSettings({ ws }: { ws: WorkspaceDto }) {
  const toast = useToast()
  const utils = trpc.useUtils()
  const [name, setName] = useState(ws.name)
  const [timezone, setTimezone] = useState(ws.timezone)
  const [prefix, setPrefix] = useState(ws.internalIdPrefix)
  const [comment, setComment] = useState(ws.comment ?? '')
  const [syncUrl, setSyncUrl] = useState((ws as { syncUrl?: string | null }).syncUrl ?? '')
  const [writeoffPhoto, setWriteoffPhoto] = useState(ws.requireWriteoffPhoto)
  const [autoNumbers, setAutoNumbers] = useState(true)
  const [flash, setFlash] = useState(false)

  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      setName(ws.name)
      setWriteoffPhoto(ws.requireWriteoffPhoto)
      setTimezone(ws.timezone)
      setPrefix(ws.internalIdPrefix)
      setComment(ws.comment ?? '')
      setSyncUrl((ws as { syncUrl?: string | null }).syncUrl ?? '')
    })
    return () => cancelAnimationFrame(frame)
  }, [ws])

  const { data: nextId } = trpc.items.nextInternalId.useQuery(
    { workspaceId: ws.id },
    { enabled: autoNumbers }
  )
  const preview = useMemo(() => {
    const numeric = nextId ? (nextId.match(/(\d+)$/)?.[1] ?? '0001') : '0001'
    return `${prefix}${numeric}`
  }, [nextId, prefix])

  const update = trpc.admin.workspaces.update.useMutation({
    onSuccess: () => {
      utils.admin.workspaces.list.invalidate()
      utils.meta.workspaces.invalidate()
      toast('Изменения сохранены')
      setFlash(true)
      window.setTimeout(() => setFlash(false), 900)
    },
    onError: (e) => toast(e.message, 'error'),
  })

  return (
    <div className={cn(cardCls, 'p-5 sm:p-6')}>
      <h3 className="mb-4 text-[17px] leading-6 font-semibold text-ink-900">
        Настройки пространства
      </h3>
      <motion.form
        animate={{ backgroundColor: flash ? '#C8FCD2' : 'rgba(200,252,210,0)' }}
        transition={{ duration: 0.35 }}
        className="-m-2 space-y-4 rounded-xl p-2"
        onSubmit={(e) => {
          e.preventDefault()
          update.mutate({
            id: ws.id,
            name: name.trim() || ws.name,
            timezone,
            internalIdPrefix: prefix,
            comment: comment.trim() || null,
            syncUrl: syncUrl.trim() || null,
            requireWriteoffPhoto: writeoffPhoto,
          })
        }}
      >
        <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
          <Field label="Название" required>
            <input className={inputCls} value={name} onChange={(e) => setName(e.target.value)} />
          </Field>
          <Field label="Часовой пояс">
            <select
              className={cn(inputCls, 'appearance-none pr-10')}
              value={timezone}
              onChange={(e) => setTimezone(e.target.value)}
            >
              {TIMEZONES.map((t) => (
                <option key={t.value} value={t.value}>
                  {t.label}
                </option>
              ))}
              {!TIMEZONES.some((t) => t.value === timezone) && (
                <option value={timezone}>{timezone}</option>
              )}
            </select>
          </Field>
          <Field label="Сервер синхронизации">
            <input
              className={inputCls}
              value={syncUrl}
              placeholder="https://sync.example.com"
              onChange={(e) => setSyncUrl(e.target.value)}
            />
          </Field>
        </div>
        <p className="-mt-2 text-[13px] text-ink-500">
          Адрес центрального сервера. Локальный узел обменивается с ним изменениями,
          когда есть связь; задаётся переменной MESHKEEPER_UPSTREAM при запуске узла.
        </p>

        <label className="flex items-start gap-2.5 text-sm text-ink-900">
          <input
            type="checkbox"
            checked={writeoffPhoto}
            onChange={(e) => setWriteoffPhoto(e.target.checked)}
            className="mt-0.5 h-4 w-4 rounded border-brand-100"
          />
          <span>
            Требовать фото при списании
            <span className="block text-[13px] text-ink-500">
              Списание не будет приниматься без снимка — в дополнение к обязательной причине.
            </span>
          </span>
        </label>

        <div className="rounded-xl border border-brand-100/70 p-4">
          <div className="mb-3 flex items-center justify-between gap-3">
            <div>
              <div className="text-sm font-semibold text-ink-900">
                Автогенерация внутренних номеров
              </div>
              <div className="text-[13px] leading-[18px] text-ink-500">
                Новым карточкам номер присваивается автоматически по шаблону
              </div>
            </div>
            <Toggle checked={autoNumbers} onChange={setAutoNumbers} />
          </div>
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-[160px_1fr]">
            <Field label="Префикс">
              <input
                className={cn(inputCls, 'font-mono-num')}
                value={prefix}
                maxLength={16}
                disabled={!autoNumbers}
                onChange={(e) => setPrefix(e.target.value)}
              />
            </Field>
            <Field label="Живой предпросмотр">
              <div className="flex h-11 items-center rounded-xl border border-dashed border-brand-100 bg-brand-50/60 px-4 text-sm text-ink-500">
                Следующий номер:&nbsp;
                <motion.span
                  key={preview}
                  initial={{ scale: 1.12, color: '#5E629B' }}
                  animate={{ scale: 1, color: '#303466' }}
                  transition={{ duration: 0.3 }}
                  className="font-mono-num"
                >
                  {preview}
                </motion.span>
              </div>
            </Field>
          </div>
        </div>

        <Field label="Комментарий">
          <textarea
            className={cn(inputCls, 'h-auto min-h-[76px] py-3 resize-y')}
            placeholder="Например: основное юрлицо, подрядные работы в СПб"
            value={comment}
            onChange={(e) => setComment(e.target.value)}
          />
        </Field>

        <div className="flex justify-end">
          <button type="submit" className={btnPrimaryCls} disabled={update.isPending}>
            {update.isPending ? 'Сохранение…' : 'Сохранить изменения'}
          </button>
        </div>
      </motion.form>
    </div>
  )
}

/* ─── Модалка создания пространства ───────────────────────────────────────── */

function CreateWorkspaceModal({ open, onClose }: { open: boolean; onClose: () => void }) {
  const toast = useToast()
  const utils = trpc.useUtils()
  const [name, setName] = useState('')
  const [timezone, setTimezone] = useState('Europe/Moscow')
  const [prefix, setPrefix] = useState('ВН-')
  const [comment, setComment] = useState('')

  const create = trpc.admin.workspaces.create.useMutation({
    onSuccess: () => {
      utils.admin.workspaces.list.invalidate()
      utils.meta.workspaces.invalidate()
      toast('Рабочее пространство создано')
      setName('')
      setTimezone('Europe/Moscow')
      setPrefix('ВН-')
      setComment('')
      onClose()
    },
    onError: (e) => toast(e.message, 'error'),
  })

  return (
    <Modal open={open} onClose={onClose} title="Создать пространство">
      <form
        className="space-y-4"
        onSubmit={(e) => {
          e.preventDefault()
          if (!name.trim()) return
          create.mutate({
            name: name.trim(),
            timezone,
            internalIdPrefix: prefix || undefined,
            comment: comment.trim() || undefined,
          })
        }}
      >
        <Field label="Название" required>
          <input
            className={inputCls}
            placeholder="ООО «Новый подрядчик»"
            value={name}
            onChange={(e) => setName(e.target.value)}
          />
        </Field>
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <Field label="Часовой пояс">
            <select
              className={cn(inputCls, 'appearance-none pr-10')}
              value={timezone}
              onChange={(e) => setTimezone(e.target.value)}
            >
              {TIMEZONES.map((t) => (
                <option key={t.value} value={t.value}>
                  {t.label}
                </option>
              ))}
            </select>
          </Field>
          <Field label="Префикс вн. номеров">
            <input
              className={cn(inputCls, 'font-mono-num')}
              value={prefix}
              maxLength={16}
              onChange={(e) => setPrefix(e.target.value)}
            />
          </Field>
        </div>
        <Field label="Комментарий">
          <input
            className={inputCls}
            placeholder="Необязательно"
            value={comment}
            onChange={(e) => setComment(e.target.value)}
          />
        </Field>
        <div className="flex justify-end gap-2 pt-1">
          <button type="button" className={btnSecondaryCls} onClick={onClose}>
            Отмена
          </button>
          <button type="submit" className={btnPrimaryCls} disabled={!name.trim() || create.isPending}>
            {create.isPending ? 'Создание…' : 'Создать'}
          </button>
        </div>
      </form>
    </Modal>
  )
}

/* ─── Раздел «Рабочие пространства» ───────────────────────────────────────── */

export default function WorkspacesSection() {
  const { workspace: storeWorkspace } = useStore()
  const { data: workspaces, isLoading } = trpc.admin.workspaces.list.useQuery()
  const { data: users } = trpc.admin.users.list.useQuery({})
  const { data: items } = trpc.reports.allItems.useQuery({})
  const [createOpen, setCreateOpen] = useState(false)
  const [inviteUrl, setInviteUrl] = useState<string | null>(null)
  const [inviteRole, setInviteRole] = useState<InviteRole>('member')
  const [inviteExpiresAt, setInviteExpiresAt] = useState<string | null>(null)
  const toast = useToast()
  const utils = trpc.useUtils()
  const syncQ = trpc.sync.status.useQuery()
  const current =
    workspaces?.find((w) => w.id === storeWorkspace?.id) ??
    workspaces?.find((w) => w.name === storeWorkspace?.name) ??
    workspaces?.[0] ??
    null
  const invitesQ = trpc.admin.workspaces.invites.useQuery(
    { workspaceId: current?.id },
    { enabled: Boolean(current?.id) },
  )
  const makeUrl = useCallback(
    (token: string) =>
      joinInviteUrl(token, {
        syncUrl: (current as { syncUrl?: string | null } | null)?.syncUrl,
        lanUrl: syncQ.data?.url,
      }),
    [current, syncQ.data?.url],
  )
  const createInvite = trpc.admin.workspaces.createInvite.useMutation({
    onSuccess: (res) => {
      if (res?.token) setInviteUrl(makeUrl(res.token))
      setInviteExpiresAt(res?.expiresAt ?? null)
      utils.admin.workspaces.invites.invalidate()
      toast('QR-приглашение готово')
    },
    onError: (e) => toast(e.message, 'error'),
  })

  const currentId = current?.id
  const issueInvite = useCallback(
    (nextRole: InviteRole) => {
      if (!currentId) return
      setInviteUrl(null)
      setInviteExpiresAt(null)
      createInvite.mutate({
        workspaceId: currentId,
        role: nextRole,
        maxUses: 50,
        expiresInHours: INVITE_TTL_HOURS,
      })
    },
    [currentId, createInvite],
  )

  useEffect(() => {
    if (!current?.id || inviteUrl) return
    const existing = firstUsableInvite(invitesQ.data)
    if (existing) {
      const frame = requestAnimationFrame(() => {
        setInviteUrl(makeUrl(existing.token))
        setInviteExpiresAt(existing.expiresAt ?? null)
      })
      return () => cancelAnimationFrame(frame)
    }
    if (invitesQ.isLoading || createInvite.isPending || createInvite.isSuccess || createInvite.isError) return
    if (invitesQ.isFetched)
      createInvite.mutate({
        workspaceId: current.id,
        role: inviteRole,
        maxUses: 50,
        expiresInHours: INVITE_TTL_HOURS,
      })
  }, [current?.id, inviteUrl, inviteRole, invitesQ.data, invitesQ.isLoading, invitesQ.isFetched, createInvite, makeUrl])

  return (
    <section>
      <SectionHeader
        title="Рабочие пространства"
        count={workspaces?.length}
        action={
          <button type="button" className={btnSecondaryCls} onClick={() => setCreateOpen(true)}>
            <Plus size={16} />
            Создать пространство
          </button>
        }
      />

      <div className="mb-4 grid grid-cols-1 gap-3 sm:grid-cols-2">
        {isLoading && (
          <div className={cn(cardCls, 'p-5 text-sm text-ink-500')}>Загрузка…</div>
        )}
        {(workspaces ?? []).map((ws) => {
          const isCurrent = current?.id === ws.id
          const metaParts: string[] = []
          if (isCurrent && items) metaParts.push(`${items.length} ед.`)
          if (isCurrent && users)
            metaParts.push(`${users.length} ${plural(users.length, ['пользователь', 'пользователя', 'пользователей'])}`)
          metaParts.push(`создано ${fmtDate(ws.createdAt)}`)
          return (
            <motion.div
              key={ws.id}
              initial={{ opacity: 0, y: 8 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ duration: 0.22 }}
              className={cn(cardCls, 'flex items-start gap-3 p-5')}
            >
              <img src="/logo-mark.svg" alt="" className="h-10 w-10 shrink-0 rounded-xl" />
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <h3 className="text-[17px] leading-6 font-semibold text-ink-900">{ws.name}</h3>
                  {isCurrent && (
                    <span className="inline-flex items-center rounded-full bg-teal/20 px-2.5 py-0.5 text-caption text-teal-dark">
                      Активно
                    </span>
                  )}
                </div>
                <div className="mt-1 text-[13px] leading-[18px] text-ink-500">
                  {metaParts.join(' · ')}
                </div>
                <div className="mt-1 flex items-center gap-2 text-[13px] leading-[18px] text-ink-500">
                  <Layers size={13} className="text-ink-300" />
                  {tzLabel(ws.timezone)} · префикс{' '}
                  <span className="font-mono-num">{ws.internalIdPrefix}</span>
                </div>
              </div>
            </motion.div>
          )
        })}
      </div>

      {current && (
        <div className={cn(cardCls, 'mb-4 p-5 sm:p-6')}>
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div>
              <h3 className="text-[17px] font-semibold text-ink-900">Пригласить по QR</h3>
              <p className="mt-1 text-sm text-ink-500">
                Новый человек сканирует код, регистрируется и сразу попадает в «{current.name}».
              </p>
            </div>
            <button
              type="button"
              className={btnPrimaryCls}
              disabled={createInvite.isPending}
              onClick={() => issueInvite(inviteRole)}
            >
              <QrCode size={16} />
              {createInvite.isPending ? 'Создание…' : inviteUrl ? 'Новый код' : 'Выпустить приглашение'}
            </button>
          </div>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            {INVITE_ROLES.map((r) => (
              <button
                key={r.value}
                type="button"
                title={r.hint}
                onClick={() => {
                  setInviteRole(r.value)
                  issueInvite(r.value)
                }}
                className={cn(
                  'h-9 rounded-xl border px-3 text-[13px] font-semibold',
                  inviteRole === r.value
                    ? 'border-accent bg-accent/10 text-accent'
                    : 'border-brand-100 bg-white text-ink-500 hover:bg-brand-50',
                )}
              >
                {r.label}
              </button>
            ))}
          </div>
          {inviteUrl && (
            <div className="mt-4 flex flex-col items-center gap-3 sm:flex-row sm:items-start">
              <InviteQrBlock value={inviteUrl} size={180} />
              <div className="min-w-0 flex-1 space-y-2">
                <p className="break-all font-mono-num text-[13px] text-ink-500">{inviteUrl}</p>
                {inviteExpiryLabel(inviteExpiresAt) && (
                  <p className="text-[13px] leading-[18px] text-ink-500">
                    Действует до {inviteExpiryLabel(inviteExpiresAt)}
                  </p>
                )}
                <button
                  type="button"
                  className={btnSecondaryCls}
                  onClick={() => {
                    void navigator.clipboard.writeText(inviteUrl)
                    toast('Ссылка скопирована')
                  }}
                >
                  Скопировать ссылку
                </button>
              </div>
            </div>
          )}
        </div>
      )}

      {current && <WorkspaceSettings ws={current} />}
      <CreateWorkspaceModal open={createOpen} onClose={() => setCreateOpen(false)} />
    </section>
  )
}
