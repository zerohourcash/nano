import { useEffect, useMemo, useRef, useState } from 'react'
import { Link, useNavigate } from 'react-router'
import { AnimatePresence, Reorder, motion } from 'framer-motion'
import { Controller, useForm } from 'react-hook-form'
import { zodResolver } from '@hookform/resolvers/zod'
import { z } from 'zod'
import {
  Building2,
  Camera,
  Check,
  CheckCircle2,
  ChevronDown,
  FileText,
  HelpCircle,
  ImagePlus,
  Info,
  Loader2,
  Minus,
  Package,
  Plus,
  QrCode,
  Trash2,
  UserRound,
  Warehouse,
  X,
} from 'lucide-react'
import { QRCodeSVG } from 'qrcode.react'
import { cn } from '@/lib/utils'
import { trpc } from '@/providers/trpc'
import { preparePhoto } from '@/lib/photo'
import type { PreparedPhoto } from '@/lib/photo'

// ─── Схема формы ─────────────────────────────────────────────────────────────

const schema = z.object({
  title: z.string().trim().min(2, 'Укажите наименование (минимум 2 символа)'),
  categoryId: z.number({ error: 'Выберите категорию' }).int().positive('Выберите категорию'),
  brandId: z.number().int().positive().nullable().optional(),
  internalIdNum: z.string().trim().regex(/^\d*$/, 'Только цифры').optional(),
  serialNumber: z.string().trim().optional(),
  cost: z.string().trim().optional(),
  quantitative: z.boolean(),
  quantity: z.number().nonnegative('Неотрицательное число').optional(),
  unit: z.string(),
  buildingSiteId: z.number().int().positive().nullable().optional(),
  storageId: z.number().int().positive().nullable().optional(),
  organizationNodeId: z.number().int().positive().nullable().optional(),
  responsibleUserId: z.number().int().positive().nullable().optional(),
  statusId: z.number().int().positive().nullable().optional(),
  comment: z.string().max(500, 'Не более 500 символов').optional(),
  ownerCompany: z.string().max(200).optional(),
  labels: z.string().max(500).optional(),
  itemType: z.string().max(100).optional(),
  currency: z.string().max(3).optional(),
  depreciationUntil: z.string().optional(),
  serviceLifeUntil: z.string().optional(),
  rentalUntil: z.string().optional(),
  supplier: z.string().max(200).optional(),
  documentNumber: z.string().max(100).optional(),
  documentDate: z.string().optional(),
  isKit: z.boolean(),
  printLabel: z.boolean(),
})

type FormValues = z.infer<typeof schema>

const UNITS = ['шт', 'м', 'кг', 'л', 'упак']

const TOOL_SAMPLES = [
  '/tool-bosch-gbh.png',
  '/tool-makita-df.png',
  '/tool-karcher-k5.png',
  '/tool-dewalt-dws.png',
  '/tool-msi-laptop.png',
  '/tool-laser-level.png',
  '/tool-metabo-grinder.png',
  '/tool-rags.png',
]

// ─── Мелкие компоненты ───────────────────────────────────────────────────────

function Toast({ text }: { text: string | null }) {
  return (
    <AnimatePresence>
      {text && (
        <motion.div
          initial={{ opacity: 0, y: 24 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 12 }}
          transition={{ duration: 0.24 }}
          className="fixed bottom-24 lg:bottom-8 left-1/2 -translate-x-1/2 z-[70] inline-flex items-center gap-2 rounded-full bg-ink-900 text-white text-sm font-semibold px-5 py-3 shadow-modal"
        >
          <CheckCircle2 size={16} className="text-teal" />
          {text}
        </motion.div>
      )}
    </AnimatePresence>
  )
}

function SectionCard({
  title,
  children,
  teal,
  delay = 0,
}: {
  title: string
  children: React.ReactNode
  teal?: boolean
  delay?: number
}) {
  return (
    <motion.section
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.28, delay, ease: 'easeOut' }}
      className={cn(
        'rounded-card p-5 sm:p-6',
        teal
          ? 'bg-[#D8F2F080] border-[1.5px] border-teal'
          : 'bg-surface border border-brand-100/60 shadow-card'
      )}
    >
      <h3 className="text-[17px] leading-6 font-semibold text-ink-900 mb-4">{title}</h3>
      {children}
    </motion.section>
  )
}

function FieldLabel({ children, required }: { children: React.ReactNode; required?: boolean }) {
  return (
    <label className="block text-[13px] font-semibold text-ink-500 mb-1.5">
      {children}
      {required && <span className="text-accent"> *</span>}
    </label>
  )
}

const inputCls =
  'w-full h-11 rounded-xl border border-brand-100 bg-surface px-3.5 text-[15px] text-ink-900 placeholder:text-ink-300 focus:border-brand-600 focus:ring-[3px] focus:ring-brand-600/15 transition-shadow'

function ErrorText({ id, message }: { id: string; message?: string }) {
  if (!message) return null
  return (
    <motion.p
      id={id}
      initial={{ x: 0 }}
      animate={{ x: [0, -5, 5, -3, 3, 0] }}
      transition={{ duration: 0.32 }}
      className="mt-1 text-xs font-semibold text-danger"
    >
      {message}
    </motion.p>
  )
}

function fmtThousands(v: string): string {
  const digits = v.replace(/[^\d]/g, '')
  if (!digits) return ''
  return new Intl.NumberFormat('ru-RU').format(Number(digits))
}

// ─── Главный компонент ───────────────────────────────────────────────────────

export default function CreateTool() {
  const navigate = useNavigate()
  const utils = trpc.useUtils()
  const [toast, setToast] = useState<string | null>(null)
  const [justCreated, setJustCreated] = useState(false)

  useEffect(() => {
    if (!toast) return
    const t = setTimeout(() => setToast(null), 4000)
    return () => clearTimeout(t)
  }, [toast])

  // Справочники
  const { data: categories } = trpc.admin.dictionaries.list.useQuery({ kind: 'categories' })
  const { data: brands } = trpc.admin.dictionaries.list.useQuery({ kind: 'brands' })
  const { data: statuses } = trpc.admin.dictionaries.list.useQuery({ kind: 'statuses' })
  const { data: storages } = trpc.admin.storages.list.useQuery({})
  const { data: sites } = trpc.admin.buildingSites.list.useQuery({})
  const { data: organizationNodes } = trpc.admin.organizationNodes.list.useQuery({})
  const { data: users } = trpc.admin.users.list.useQuery({})
  const { data: workspaces } = trpc.meta.workspaces.useQuery()
  const { data: nextId } = trpc.items.nextInternalId.useQuery({})

  const workspaceName = workspaces?.[0]?.name ?? 'ООО «СтройМонтаж»'
  const prefix = useMemo(() => (nextId ? (nextId.match(/^\D*/)?.[0] ?? 'ВН-') : 'ВН-'), [nextId])
  const nextNum = useMemo(() => (nextId ? nextId.slice(prefix.length) : ''), [nextId, prefix])

  const {
    register,
    handleSubmit,
    control,
    watch,
    setValue,
    reset,
    getValues,
    setFocus,
    formState: { errors },
  } = useForm<FormValues>({
    resolver: zodResolver(schema),
    defaultValues: {
      title: '',
      serialNumber: '',
      cost: '',
      quantitative: false,
      unit: 'шт',
      internalIdNum: '',
      comment: '',
      ownerCompany: '',
      labels: '',
      itemType: 'Инструмент',
      currency: 'RUB',
      depreciationUntil: '',
      serviceLifeUntil: '',
      rentalUntil: '',
      supplier: '',
      documentNumber: '',
      documentDate: '',
      isKit: false,
      printLabel: false,
    },
  })

  // Автозаполнение следующего внутреннего номера. Ответственного не назначаем:
  // складской предмет должен оставаться свободным до явной выдачи.
  useEffect(() => {
    if (nextNum && !getValues('internalIdNum')) setValue('internalIdNum', nextNum)
  }, [nextNum, getValues, setValue])

  useEffect(() => {
    if (statuses?.length && !getValues('statusId')) {
      const inStock = statuses.find((s) => s.slug === 'in-stock') ?? statuses[0]
      setValue('statusId', inStock.id)
    }
  }, [statuses, getValues, setValue])

  const title = watch('title')
  const categoryId = watch('categoryId')
  const internalIdNum = watch('internalIdNum')
  const comment = watch('comment') ?? ''

  // Фото сначала загружаются в локальный CAS, затем карточка получает маленькую
  // подписанную ссылку cas:sha256. Base64 никогда не попадает в Ledger.
  const [titlePhoto, setTitlePhoto] = useState<PreparedPhoto | null>(null)
  const [extraPhotos, setExtraPhotos] = useState<PreparedPhoto[]>([])
  const [samplesOpen, setSamplesOpen] = useState(false)
  const titleFileRef = useRef<HTMLInputElement>(null)
  const extraFileRef = useRef<HTMLInputElement>(null)

  // Байты остаются локальными до создания карточки, затем попадают в CAS.
  const [docs, setDocs] = useState<{ name: string; size: number; file: File }[]>([])
  const docFileRef = useRef<HTMLInputElement>(null)

  // Автоподсказки брендов при вводе наименования
  const brandHints = useMemo(() => {
    const q = title.trim().toLowerCase()
    if (q.length < 2 || !brands) return []
    return brands.filter((b) => b.name.toLowerCase().includes(q) && !q.includes(b.name.toLowerCase())).slice(0, 5)
  }, [title, brands])

  const create = trpc.items.create.useMutation()
  const ingestContent = trpc.content.ingest.useMutation()
  const addPhoto = trpc.items.addPhoto.useMutation()
  const addDocument = trpc.items.addDocument.useMutation()

  const doSubmit = (values: FormValues, andMore: boolean) => {
    const costNum = values.cost ? Number(values.cost.replace(/[^\d]/g, '')) : undefined
    const internalId =
      prefix + (values.internalIdNum && values.internalIdNum.length > 0 ? values.internalIdNum : nextNum)
    const photos = [titlePhoto, ...extraPhotos].filter((p): p is PreparedPhoto => Boolean(p))
    const metadata = {
      ownerCompany: values.ownerCompany?.trim() || undefined,
      labels: values.labels?.split(',').map((x) => x.trim()).filter(Boolean) ?? [],
      itemType: values.itemType?.trim() || undefined,
      currency: values.currency?.trim().toUpperCase() || 'RUB',
      depreciationUntil: values.depreciationUntil || undefined,
      serviceLifeUntil: values.serviceLifeUntil || undefined,
      rentalUntil: values.rentalUntil || undefined,
      supplier: values.supplier?.trim() || undefined,
      documentNumber: values.documentNumber?.trim() || undefined,
      documentDate: values.documentDate || undefined,
      isKit: values.isKit,
    }

    create.mutate(
      {
        title: values.title.trim(),
        categoryId: values.categoryId,
        brandId: values.brandId ?? undefined,
        internalId: internalId || undefined,
        serialNumber: values.serialNumber || undefined,
        cost: costNum,
        quantitative: values.quantitative,
        quantity: values.quantitative ? values.quantity ?? 0 : undefined,
        unit: values.quantitative ? values.unit : undefined,
        buildingSiteId: values.buildingSiteId ?? undefined,
        storageId: values.storageId ?? undefined,
        organizationNodeId: values.organizationNodeId ?? undefined,
        responsibleUserId: values.responsibleUserId ?? undefined,
        statusId: values.statusId ?? undefined,
        comment: values.comment?.trim() || undefined,
        metadata,
        qrCode: internalId || undefined,
      },
      {
        onSuccess: async (item) => {
          try {
            if (item) {
              const itemGuid = (item as typeof item & { guid?: string }).guid
              if (!itemGuid) throw new Error('Нода не вернула GUID карточки')
              for (const [index, photo] of photos.entries()) {
                const [original, thumbnail] = await Promise.all([
                  ingestContent.mutateAsync({ workspaceId: item.workspaceId, dataUrl: photo.url }),
                  ingestContent.mutateAsync({ workspaceId: item.workspaceId, dataUrl: photo.thumbUrl }),
                ])
                await addPhoto.mutateAsync({
                  itemId: item.id,
                  itemGuid,
                  photoGuid: crypto.randomUUID(),
                  url: original.url,
                  thumbUrl: thumbnail.url,
                  isTitle: index === 0,
                })
              }
              for (const document of docs) {
                const dataUrl = await new Promise<string>((resolve, reject) => {
                  const reader = new FileReader()
                  reader.onerror = () => reject(reader.error ?? new Error('Не удалось прочитать документ'))
                  reader.onload = () => resolve(String(reader.result))
                  reader.readAsDataURL(document.file)
                })
                const uploaded = await ingestContent.mutateAsync({ workspaceId: item.workspaceId, dataUrl })
                await addDocument.mutateAsync({
                  itemId: item.id,
                  itemGuid,
                  documentGuid: crypto.randomUUID(),
                  name: document.name,
                  url: uploaded.url,
                  mime: uploaded.mime,
                  accessLevel: 'members',
                })
              }
            }
          } catch (error) {
            setToast(error instanceof Error ? `Карточка создана, но вложение не добавлено: ${error.message}` : 'Карточка создана, но вложение не добавлено')
          }
          utils.items.list.invalidate()
          utils.items.nextInternalId.invalidate()
          if (andMore) {
            reset()
            setTitlePhoto(null)
            setExtraPhotos([])
            setDocs([])
            setToast(`Инструмент ${item?.internalId ?? ''} создан — можно добавить следующий`)
            requestAnimationFrame(() => setFocus('title'))
            // заново подставить дефолты
            if (statuses?.length) {
              const inStock = statuses.find((s) => s.slug === 'in-stock') ?? statuses[0]
              setValue('statusId', inStock.id)
            }
          } else {
            setJustCreated(true)
            setToast(`Инструмент ${item?.internalId ?? ''} создан. QR готов к печати`)
            setTimeout(() => {
              if (item) navigate(`/tool/${item.id}`)
            }, 700)
          }
        },
        onError: (e) => setToast(e.message || 'Не удалось создать инструмент'),
      }
    )
  }

  const onInvalid = () => {
    const first = document.querySelector('[data-error="true"]')
    first?.scrollIntoView({ behavior: 'smooth', block: 'center' })
  }

  const requiredDone = [title.trim().length >= 2, categoryId != null].filter(Boolean).length

  return (
    <div className="space-y-5 pb-20 lg:pb-0">
      {/* Секция 0. Заголовок */}
      <motion.div
        initial={{ opacity: 0, y: 16 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.26 }}
        className="space-y-1"
      >
        <nav className="text-[13px] text-ink-500 flex items-center gap-1.5">
          <Link to="/" className="hover:text-brand-600 transition-colors">
            Каталог
          </Link>
          <span>/</span>
          <span className="text-ink-900">Создание</span>
        </nav>
        <h1 className="text-2xl lg:text-[28px] leading-9 font-bold tracking-[-0.01em] text-ink-900">
          Новый инструмент
        </h1>
        <p className="text-[13px] text-ink-500">
          Карточка будет создана в рабочем пространстве <span className="font-semibold">{workspaceName}</span>
        </p>
      </motion.div>

      <form
        onSubmit={handleSubmit((v) => doSubmit(v, false), onInvalid)}
        className="grid lg:grid-cols-[minmax(0,720px)_280px] gap-6 items-start"
      >
        {/* Левая колонка: секции формы */}
        <div className="space-y-5">
          {/* Секция 1. Основное */}
          <SectionCard title="Основная информация">
            <div className="grid sm:grid-cols-2 gap-4">
              <div className="sm:col-span-2 relative" data-error={Boolean(errors.title) || undefined}>
                <FieldLabel required>Наименование</FieldLabel>
                <input
                  {...register('title')}
                  placeholder="Например: Перфоратор Bosch GBH 8-45 DV"
                  className={cn(inputCls, errors.title && 'border-danger')}
                  autoComplete="off"
                />
                {brandHints.length > 0 && (
                  <div className="absolute z-10 left-0 right-0 top-full mt-1 bg-surface rounded-xl border border-brand-100 shadow-hover py-1">
                    {brandHints.map((b) => (
                      <button
                        key={b.id}
                        type="button"
                        onClick={() => {
                          setValue('title', `${title.trim()} ${b.name}`.replace(/\s+/g, ' '), { shouldValidate: true })
                          setValue('brandId', b.id)
                        }}
                        className="w-full text-left px-3.5 py-2 text-sm text-ink-900 hover:bg-brand-50 transition-colors"
                      >
                        + бренд <span className="font-semibold">{b.name}</span>
                      </button>
                    ))}
                  </div>
                )}
                <ErrorText id="err-title" message={errors.title?.message} />
              </div>

              <div data-error={Boolean(errors.categoryId) || undefined}>
                <FieldLabel required>Категория</FieldLabel>
                <Controller
                  control={control}
                  name="categoryId"
                  render={({ field }) => (
                    <select
                      value={field.value ?? ''}
                      onChange={(e) => field.onChange(e.target.value ? Number(e.target.value) : undefined)}
                      className={cn(inputCls, errors.categoryId && 'border-danger')}
                    >
                      <option value="">Выберите категорию</option>
                      {(categories ?? []).map((c) => (
                        <option key={c.id} value={c.id}>
                          {c.name}
                        </option>
                      ))}
                    </select>
                  )}
                />
                <ErrorText id="err-category" message={errors.categoryId?.message} />
              </div>

              <div>
                <FieldLabel>Бренд</FieldLabel>
                <Controller
                  control={control}
                  name="brandId"
                  render={({ field }) => (
                    <select
                      value={field.value ?? ''}
                      onChange={(e) => field.onChange(e.target.value ? Number(e.target.value) : null)}
                      className={inputCls}
                    >
                      <option value="">—</option>
                      {(brands ?? []).map((b) => (
                        <option key={b.id} value={b.id}>
                          {b.name}
                        </option>
                      ))}
                    </select>
                  )}
                />
              </div>

              <div>
                <FieldLabel>Вн. номер</FieldLabel>
                <div className="flex h-11 rounded-xl border border-brand-100 bg-surface overflow-hidden focus-within:border-brand-600 focus-within:ring-[3px] focus-within:ring-brand-600/15 transition-shadow">
                  <span className="inline-flex items-center px-3 bg-brand-50 border-r border-brand-100 font-mono-num text-ink-500 select-none">
                    {prefix}
                  </span>
                  <input
                    {...register('internalIdNum')}
                    inputMode="numeric"
                    placeholder={nextNum}
                    className="flex-1 min-w-0 px-3 font-mono-num text-ink-900 placeholder:text-ink-300 bg-transparent"
                  />
                </div>
                <p className="mt-1 text-xs text-ink-300">
                  Автогенерация по шаблону рабочего пространства — можно изменить
                </p>
                <ErrorText id="err-internal" message={errors.internalIdNum?.message} />
              </div>

              <div>
                <FieldLabel>Серийный номер</FieldLabel>
                <input {...register('serialNumber')} className={cn(inputCls, 'font-mono-num')} placeholder="—" />
              </div>

              <div>
                <FieldLabel>Стоимость, ₽</FieldLabel>
                <Controller
                  control={control}
                  name="cost"
                  render={({ field }) => (
                    <input
                      value={field.value ?? ''}
                      inputMode="numeric"
                      placeholder="0"
                      onChange={(e) => field.onChange(e.target.value.replace(/[^\d\s]/g, ''))}
                      onBlur={() => field.onChange(field.value ? fmtThousands(field.value) : '')}
                      onFocus={() => field.onChange((field.value ?? '').replace(/\s/g, ''))}
                      className={cn(inputCls, 'font-mono-num')}
                    />
                  )}
                />
              </div>

              {/* Чекбокс-карточка «Материалы и расходники» */}
              <div className="sm:col-span-2">
                <Controller
                  control={control}
                  name="quantitative"
                  render={({ field }) => (
                    <div
                      className={cn(
                        'rounded-xl border-[1.5px] transition-colors duration-200',
                        field.value ? 'border-brand-600 bg-brand-50' : 'border-brand-100 bg-surface'
                      )}
                    >
                      <button
                        type="button"
                        onClick={() => field.onChange(!field.value)}
                        className="w-full flex items-center gap-3 px-4 py-3.5 text-left"
                      >
                        <span
                          className={cn(
                            'w-5 h-5 shrink-0 rounded-md border-[1.5px] flex items-center justify-center transition-colors',
                            field.value ? 'bg-brand-600 border-brand-600' : 'border-brand-100 bg-surface'
                          )}
                        >
                          {field.value && <Check size={13} strokeWidth={3} className="text-white" />}
                        </span>
                        <Package size={18} strokeWidth={1.75} className="text-brand-600 shrink-0" />
                        <span className="text-sm font-semibold text-ink-900">Материалы и расходники</span>
                        <span className="text-xs text-ink-500 ml-auto hidden sm:block">количественный учёт</span>
                      </button>
                      <AnimatePresence initial={false}>
                        {field.value && (
                          <motion.div
                            initial={{ height: 0, opacity: 0 }}
                            animate={{ height: 'auto', opacity: 1 }}
                            exit={{ height: 0, opacity: 0 }}
                            transition={{ duration: 0.24, ease: 'easeOut' }}
                            className="overflow-hidden"
                          >
                            <div className="px-4 pb-4 pt-1 grid sm:grid-cols-2 gap-4">
                              <div>
                                <FieldLabel required>Количество</FieldLabel>
                                <div className="flex items-center gap-2">
                                  <button
                                    type="button"
                                    aria-label="Меньше"
                                    onClick={() =>
                                      setValue('quantity', Math.max(0, (getValues('quantity') ?? 0) - 1))
                                    }
                                    className="w-11 h-11 shrink-0 rounded-xl border border-brand-100 bg-surface flex items-center justify-center text-ink-900 hover:bg-brand-50 transition-colors"
                                  >
                                    <Minus size={16} strokeWidth={1.75} />
                                  </button>
                                  <Controller
                                    control={control}
                                    name="quantity"
                                    render={({ field: f }) => (
                                      <input
                                        value={f.value ?? ''}
                                        inputMode="decimal"
                                        onChange={(e) =>
                                          f.onChange(e.target.value === '' ? undefined : Number(e.target.value.replace(',', '.')))
                                        }
                                        className={cn(inputCls, 'text-center font-mono-num')}
                                        placeholder="0"
                                      />
                                    )}
                                  />
                                  <button
                                    type="button"
                                    aria-label="Больше"
                                    onClick={() => setValue('quantity', (getValues('quantity') ?? 0) + 1)}
                                    className="w-11 h-11 shrink-0 rounded-xl border border-brand-100 bg-surface flex items-center justify-center text-ink-900 hover:bg-brand-50 transition-colors"
                                  >
                                    <Plus size={16} strokeWidth={1.75} />
                                  </button>
                                </div>
                                <ErrorText id="err-qty" message={errors.quantity?.message} />
                              </div>
                              <div>
                                <FieldLabel>Ед. изм.</FieldLabel>
                                <Controller
                                  control={control}
                                  name="unit"
                                  render={({ field: f }) => (
                                    <select value={f.value} onChange={f.onChange} className={inputCls}>
                                      {UNITS.map((u) => (
                                        <option key={u} value={u}>
                                          {u}
                                        </option>
                                      ))}
                                    </select>
                                  )}
                                />
                              </div>
                              <div className="sm:col-span-2 rounded-xl bg-info-bg border-l-[3px] border-teal px-3 py-2.5 text-sm text-ink-900 flex gap-2">
                                <Info size={15} strokeWidth={1.75} className="shrink-0 mt-0.5 text-teal-dark" />
                                Количественный учёт: остаток меняется пополнением и списанием, частичная передача
                                делит партию.
                              </div>
                            </div>
                          </motion.div>
                        )}
                      </AnimatePresence>
                    </div>
                  )}
                />
              </div>
            </div>
          </SectionCard>

          {/* Секция 2. Размещение и ответственность */}
          <SectionCard title="Где находится" delay={0.05}>
            <div className="grid sm:grid-cols-2 gap-4">
              <div className="sm:col-span-2">
                <FieldLabel>Раздел структуры</FieldLabel>
                <Controller
                  control={control}
                  name="organizationNodeId"
                  render={({ field }) => (
                    <select
                      value={field.value ?? ''}
                      onChange={(e) => field.onChange(e.target.value ? Number(e.target.value) : null)}
                      className={inputCls}
                    >
                      <option value="">Не выбран</option>
                      {(organizationNodes ?? []).map((node) => (
                        <option key={node.id} value={node.id}>
                          {node.name} · {node.kind}
                        </option>
                      ))}
                    </select>
                  )}
                />
              </div>
              <div>
                <FieldLabel>Объект</FieldLabel>
                <div className="relative">
                  <Building2 size={16} strokeWidth={1.75} className="absolute left-3.5 top-1/2 -translate-y-1/2 text-ink-300 pointer-events-none" />
                  <Controller
                    control={control}
                    name="buildingSiteId"
                    render={({ field }) => (
                      <select
                        value={field.value ?? ''}
                        onChange={(e) => field.onChange(e.target.value ? Number(e.target.value) : null)}
                        className={cn(inputCls, 'pl-10')}
                      >
                        <option value="">—</option>
                        {(sites ?? []).map((s) => (
                          <option key={s.id} value={s.id}>
                            {s.name}
                          </option>
                        ))}
                      </select>
                    )}
                  />
                </div>
              </div>
              <div>
                <FieldLabel>Склад</FieldLabel>
                <div className="relative">
                  <Warehouse size={16} strokeWidth={1.75} className="absolute left-3.5 top-1/2 -translate-y-1/2 text-ink-300 pointer-events-none" />
                  <Controller
                    control={control}
                    name="storageId"
                    render={({ field }) => (
                      <select
                        value={field.value ?? ''}
                        onChange={(e) => field.onChange(e.target.value ? Number(e.target.value) : null)}
                        className={cn(inputCls, 'pl-10')}
                      >
                        <option value="">—</option>
                        {(storages ?? []).map((s) => (
                          <option key={s.id} value={s.id}>
                            {s.name}
                          </option>
                        ))}
                      </select>
                    )}
                  />
                </div>
              </div>
              <div className="sm:col-span-2">
                <FieldLabel>Ответственный</FieldLabel>
                <div className="relative">
                  <UserRound size={16} strokeWidth={1.75} className="absolute left-3.5 top-1/2 -translate-y-1/2 text-ink-300 pointer-events-none" />
                  <Controller
                    control={control}
                    name="responsibleUserId"
                    render={({ field }) => (
                      <select
                        value={field.value ?? ''}
                        onChange={(e) => field.onChange(e.target.value ? Number(e.target.value) : null)}
                        className={cn(inputCls, 'pl-10')}
                      >
                        <option value="">—</option>
                        {(users ?? [])
                          .filter((u) => u.status === 'active')
                          .map((u) => (
                            <option key={u.id} value={u.id}>
                              {u.fullName}
                              {u.position ? ` · ${u.position}` : ''}
                            </option>
                          ))}
                      </select>
                    )}
                  />
                </div>
              </div>
              <div className="sm:col-span-2">
                <FieldLabel>Статус</FieldLabel>
                <Controller
                  control={control}
                  name="statusId"
                  render={({ field }) => (
                    <div className="flex flex-wrap gap-1.5 rounded-xl border border-brand-100 bg-surface p-1.5">
                      {(statuses ?? []).map((s) => {
                        const active = field.value === s.id
                        return (
                          <button
                            key={s.id}
                            type="button"
                            onClick={() => field.onChange(s.id)}
                            className={cn(
                              'relative flex items-center gap-1.5 h-9 px-3.5 rounded-lg text-[13px] font-semibold transition-colors',
                              active ? 'text-ink-900' : 'text-ink-500 hover:text-ink-900'
                            )}
                          >
                            {active && (
                              <motion.span
                                layoutId="create-status-pill"
                                className="absolute inset-0 rounded-lg"
                                style={{ background: s.bg ?? '#EDEDF7' }}
                                transition={{ duration: 0.2 }}
                              />
                            )}
                            <span className="relative w-2 h-2 rounded-full" style={{ background: s.color ?? '#6B6E9E' }} />
                            <span className="relative">{s.name}</span>
                          </button>
                        )
                      })}
                    </div>
                  )}
                />
              </div>
            </div>
          </SectionCard>

          {/* Расширенная карточка по модели FaceKit */}
          <SectionCard title="Учётные и закупочные данные" delay={0.08}>
            <div className="grid sm:grid-cols-2 gap-4">
              <div>
                <FieldLabel>Компания-владелец</FieldLabel>
                <input {...register('ownerCompany')} className={inputCls} placeholder="Наименование организации" />
              </div>
              <div>
                <FieldLabel>Тип ТМЦ</FieldLabel>
                <input {...register('itemType')} className={inputCls} placeholder="Инструмент" />
              </div>
              <div className="sm:col-span-2">
                <FieldLabel>Ярлыки</FieldLabel>
                <input {...register('labels')} className={inputCls} placeholder="Через запятую" />
              </div>
              <div>
                <FieldLabel>Амортизация до</FieldLabel>
                <input {...register('depreciationUntil')} type="date" className={inputCls} />
              </div>
              <div>
                <FieldLabel>Срок службы до</FieldLabel>
                <input {...register('serviceLifeUntil')} type="date" className={inputCls} />
              </div>
              <div>
                <FieldLabel>Аренда до</FieldLabel>
                <input {...register('rentalUntil')} type="date" className={inputCls} />
              </div>
              <div>
                <FieldLabel>Валюта</FieldLabel>
                <input {...register('currency')} maxLength={3} className={inputCls} />
              </div>
              <div>
                <FieldLabel>Поставщик</FieldLabel>
                <input {...register('supplier')} className={inputCls} />
              </div>
              <div>
                <FieldLabel>Номер документа</FieldLabel>
                <input {...register('documentNumber')} className={inputCls} />
              </div>
              <div>
                <FieldLabel>Дата документа</FieldLabel>
                <input {...register('documentDate')} type="date" className={inputCls} />
              </div>
              <label className="flex items-center gap-3 self-end h-11 text-sm font-medium text-ink-900">
                <input {...register('isKit')} type="checkbox" className="h-4 w-4 accent-brand-600" />
                Это комплект
              </label>
            </div>
          </SectionCard>

          {/* Секция 3. Фото */}
          <SectionCard title="Фотографии" delay={0.1}>
            <div className="space-y-4">
              {/* Титульное фото */}
              {titlePhoto ? (
                <motion.div
                  initial={{ scale: 0.9, opacity: 0 }}
                  animate={{ scale: 1, opacity: 1 }}
                  transition={{ type: 'spring', stiffness: 300, damping: 24 }}
                  className="relative aspect-[4/3] max-w-[420px] rounded-xl overflow-hidden bg-brand-50 border border-brand-100/60"
                >
                  <img src={titlePhoto.url} alt="Титульное фото" className="w-full h-full object-cover" />
                  <div className="absolute right-2 top-2 flex gap-1.5">
                    <button
                      type="button"
                      onClick={() => titleFileRef.current?.click()}
                      className="h-8 px-3 rounded-lg bg-surface/90 backdrop-blur-sm text-xs font-semibold text-ink-900 shadow-card hover:bg-surface transition-colors"
                    >
                      Заменить
                    </button>
                    <button
                      type="button"
                      onClick={() => setTitlePhoto(null)}
                      aria-label="Удалить фото"
                      className="w-8 h-8 rounded-lg bg-surface/90 backdrop-blur-sm text-danger shadow-card hover:bg-surface transition-colors flex items-center justify-center"
                    >
                      <Trash2 size={14} strokeWidth={1.75} />
                    </button>
                  </div>
                </motion.div>
              ) : (
                <button
                  type="button"
                  onClick={() => titleFileRef.current?.click()}
                  className="w-full aspect-[4/3] max-w-[420px] rounded-xl border border-dashed border-brand-100 bg-surface hover:bg-brand-50 transition-colors flex flex-col items-center justify-center gap-2 text-ink-300"
                >
                  <Camera size={32} strokeWidth={1.5} />
                  <span className="text-sm font-semibold text-ink-500">Перетащите фото или нажмите</span>
                  <span className="text-xs">Титульное фото 4:3</span>
                </button>
              )}
              <input
                ref={titleFileRef}
                type="file"
                accept="image/*"
                className="hidden"
                onChange={(e) => {
                  const f = e.target.files?.[0]
                  if (f) void preparePhoto(f).then(setTitlePhoto)
                  e.target.value = ''
                }}
              />

              {/* Дополнительные фото (Reorder) */}
              <div>
                <div className="text-[13px] font-semibold text-ink-500 mb-2">Дополнительные</div>
                <Reorder.Group
                  axis="x"
                  values={extraPhotos}
                  onReorder={setExtraPhotos}
                  className="flex flex-wrap gap-2"
                >
                  {extraPhotos.map((src) => (
                    <Reorder.Item
                      key={src.url}
                      value={src}
                      whileDrag={{ scale: 1.06, boxShadow: '0 12px 32px rgba(48,52,102,.14)' }}
                      className="relative w-24 h-[72px] rounded-lg overflow-hidden bg-brand-50 border border-brand-100/60 cursor-grab active:cursor-grabbing"
                    >
                      <img
                        src={src.thumbUrl}
                        alt=""
                        className="w-full h-full object-cover pointer-events-none"
                      />
                      <button
                        type="button"
                        aria-label="Убрать фото"
                        onClick={() => setExtraPhotos((p) => p.filter((x) => x !== src))}
                        className="absolute right-1 top-1 w-5 h-5 rounded-md bg-surface/90 text-danger flex items-center justify-center"
                      >
                        <X size={11} strokeWidth={2} />
                      </button>
                    </Reorder.Item>
                  ))}
                  <button
                    type="button"
                    onClick={() => extraFileRef.current?.click()}
                    aria-label="Добавить фото"
                    className="w-24 h-[72px] rounded-lg border border-dashed border-brand-100 flex items-center justify-center text-ink-300 hover:bg-brand-50 hover:text-brand-600 transition-colors"
                  >
                    <Plus size={18} strokeWidth={1.75} />
                  </button>
                </Reorder.Group>
                <input
                  ref={extraFileRef}
                  type="file"
                  accept="image/*"
                  multiple
                  className="hidden"
                  onChange={(e) => {
                    const files = Array.from(e.target.files ?? [])
                    if (files.length) {
                      void Promise.all(files.map(preparePhoto)).then((prepared) =>
                        setExtraPhotos((p) => [...p, ...prepared]),
                      )
                    }
                    e.target.value = ''
                  }}
                />
              </div>

              {/* Галерея образцов */}
              <div>
                <button
                  type="button"
                  onClick={() => setSamplesOpen((v) => !v)}
                  className="inline-flex items-center gap-1.5 text-[13px] font-semibold text-brand-600 hover:text-brand-700 transition-colors"
                >
                  <ImagePlus size={14} strokeWidth={1.75} />
                  Выбрать из галереи образцов
                  <ChevronDown
                    size={14}
                    strokeWidth={1.75}
                    className={cn('transition-transform duration-200', samplesOpen && 'rotate-180')}
                  />
                </button>
                <AnimatePresence initial={false}>
                  {samplesOpen && (
                    <motion.div
                      initial={{ height: 0, opacity: 0 }}
                      animate={{ height: 'auto', opacity: 1 }}
                      exit={{ height: 0, opacity: 0 }}
                      transition={{ duration: 0.24, ease: 'easeOut' }}
                      className="overflow-hidden"
                    >
                      <div className="grid grid-cols-4 sm:grid-cols-8 gap-2 pt-3">
                        {TOOL_SAMPLES.map((src) => (
                          <button
                            key={src}
                            type="button"
                            onClick={() => {
                              // Образцы уже лежат статикой, уменьшать нечего.
                              const sample = { url: src, thumbUrl: src }
                              if (!titlePhoto) setTitlePhoto(sample)
                              else if (!extraPhotos.some((p) => p.url === src))
                                setExtraPhotos((p) => [...p, sample])
                              setSamplesOpen(false)
                            }}
                            className="aspect-[4/3] rounded-lg overflow-hidden bg-brand-50 border border-brand-100/60 hover:ring-2 hover:ring-brand-600/40 transition-shadow"
                          >
                            <img src={src} alt="" className="w-full h-full object-cover" loading="lazy" />
                          </button>
                        ))}
                      </div>
                    </motion.div>
                  )}
                </AnimatePresence>
              </div>
            </div>
          </SectionCard>

          {/* Секция 4. Документы и комментарий */}
          <SectionCard title="Документы и комментарий" delay={0.15}>
            <div className="space-y-4">
              <div>
                <button
                  type="button"
                  onClick={() => docFileRef.current?.click()}
                  className="w-full rounded-xl border border-dashed border-brand-100 px-4 py-4 text-sm font-semibold text-ink-500 hover:bg-brand-50 transition-colors flex items-center justify-center gap-2"
                >
                  <FileText size={16} strokeWidth={1.75} />
                  Добавить документ
                </button>
                <input
                  ref={docFileRef}
                  type="file"
                  multiple
                  className="hidden"
                  onChange={(e) => {
                    const files = Array.from(e.target.files ?? [])
                    if (files.length) setDocs((p) => [...p, ...files.map((file) => ({ name: file.name, size: file.size, file }))])
                    e.target.value = ''
                  }}
                />
                {docs.length > 0 && (
                  <ul className="mt-2 space-y-1.5">
                    {docs.map((d, i) => (
                      <li
                        key={`${d.name}-${i}`}
                        className="flex items-center gap-2.5 rounded-xl border border-brand-100/60 bg-surface px-3 py-2"
                      >
                        <FileText size={15} strokeWidth={1.75} className="text-brand-600 shrink-0" />
                        <span className="text-sm font-semibold text-ink-900 truncate flex-1">{d.name}</span>
                        <span className="font-mono-num text-ink-500 shrink-0">
                          {(d.size / 1024).toFixed(0)} КБ
                        </span>
                        <button
                          type="button"
                          aria-label="Убрать документ"
                          onClick={() => setDocs((p) => p.filter((_, j) => j !== i))}
                          className="w-7 h-7 shrink-0 rounded-lg text-ink-300 hover:text-danger hover:bg-danger-bg/50 flex items-center justify-center transition-colors"
                        >
                          <X size={13} strokeWidth={2} />
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
                <p className="mt-1.5 text-xs text-ink-300">
                  В демо-версии файлы прикрепляются к карточке через панель управления
                </p>
              </div>
              <div>
                <FieldLabel>Комментарий</FieldLabel>
                <textarea
                  {...register('comment')}
                  rows={3}
                  maxLength={500}
                  placeholder="Особенности, комплектация, состояние…"
                  className="w-full rounded-xl border border-brand-100 bg-surface px-3.5 py-2.5 text-[15px] text-ink-900 placeholder:text-ink-300 focus:border-brand-600 focus:ring-[3px] focus:ring-brand-600/15 resize-none"
                />
                <div className="flex justify-between mt-1">
                  <ErrorText id="err-comment" message={errors.comment?.message} />
                  <span className="ml-auto font-mono-num text-ink-300">{comment.length}/500</span>
                </div>
              </div>
            </div>
          </SectionCard>

          {/* Секция 5. QR-код */}
          <SectionCard title="QR-код" teal delay={0.2}>
            <div className="flex flex-col sm:flex-row items-center gap-4">
              <motion.div
                initial={{ opacity: 0 }}
                whileInView={{ opacity: 1 }}
                viewport={{ once: true, amount: 0.4 }}
                transition={{ duration: 0.4 }}
                className="shrink-0 bg-white rounded-xl p-2.5 border border-teal/40"
              >
                <QRCodeSVG value={prefix + (internalIdNum || nextNum || '0000')} size={96} level="M" />
              </motion.div>
              <div className="space-y-2.5 text-center sm:text-left">
                <p className="text-sm text-ink-900 flex items-center justify-center sm:justify-start gap-2">
                  <QrCode size={15} strokeWidth={1.75} className="text-teal-dark" />
                  QR-код сформируется автоматически после создания карточки
                </p>
                <label className="inline-flex items-center gap-2.5 cursor-pointer select-none">
                  <Controller
                    control={control}
                    name="printLabel"
                    render={({ field }) => (
                      <button
                        type="button"
                        role="checkbox"
                        aria-checked={field.value}
                        onClick={() => field.onChange(!field.value)}
                        className={cn(
                          'w-5 h-5 shrink-0 rounded-md border-[1.5px] flex items-center justify-center transition-colors',
                          field.value ? 'bg-brand-600 border-brand-600' : 'border-brand-100 bg-surface'
                        )}
                      >
                        {field.value && <Check size={13} strokeWidth={3} className="text-white" />}
                      </button>
                    )}
                  />
                  <span className="text-sm text-ink-900">Распечатать этикетку сразу после создания</span>
                </label>
              </div>
            </div>
          </SectionCard>
        </div>

        {/* Секция 6. Правая колонка-помощник */}
        <motion.aside
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.28, delay: 0.1 }}
          className="hidden lg:block sticky top-20 rounded-card bg-brand-50 border border-brand-100/60 p-5 space-y-5"
        >
          <h3 className="text-[17px] leading-6 font-semibold text-ink-900">Подсказки</h3>
          <div className="space-y-1">
            {[
              {
                q: 'Как работает вн. номер',
                a: 'Номер формируется по шаблону рабочего пространства (префикс + порядковый номер). Можно изменить вручную — главное, чтобы он был уникальным.',
              },
              {
                q: 'Чем материал отличается от инструмента',
                a: 'Материал учитывается количественно: есть остаток, пополнение и списание. Инструмент — штучная единица с ответственным.',
              },
              {
                q: 'Зачем нужен QR',
                a: 'Наклейте этикетку на корпус: сканирование сразу открывает карточку — удобно при выдаче и инвентаризации.',
              },
            ].map((t) => (
              <details key={t.q} className="group rounded-xl bg-surface/70 border border-brand-100/40">
                <summary className="flex items-center gap-2 px-3 py-2.5 cursor-pointer text-sm font-semibold text-ink-900 list-none">
                  <HelpCircle size={14} strokeWidth={1.75} className="text-brand-600 shrink-0" />
                  {t.q}
                  <ChevronDown
                    size={14}
                    strokeWidth={1.75}
                    className="ml-auto text-ink-300 transition-transform duration-200 group-open:rotate-180"
                  />
                </summary>
                <p className="px-3 pb-3 text-[13px] leading-[18px] text-ink-500">{t.a}</p>
              </details>
            ))}
          </div>
          <div className="rounded-xl bg-surface/70 border border-brand-100/40 p-3">
            <div className="text-[13px] font-semibold text-ink-500 mb-2">
              Обязательные поля: {requiredDone} из 2
            </div>
            {[
              { ok: title.trim().length >= 2, label: 'Наименование' },
              { ok: categoryId != null, label: 'Категория' },
            ].map((c) => (
              <div key={c.label} className="flex items-center gap-2 py-1">
                <span
                  className={cn(
                    'w-[18px] h-[18px] rounded-full flex items-center justify-center transition-colors',
                    c.ok ? 'bg-success' : 'bg-brand-100/60'
                  )}
                >
                  {c.ok && <Check size={11} strokeWidth={3} className="text-white" />}
                </span>
                <span
                  className={cn(
                    'text-sm transition-colors',
                    c.ok ? 'text-success font-semibold' : 'text-ink-500'
                  )}
                >
                  {c.label}
                </span>
              </div>
            ))}
          </div>
        </motion.aside>

        {/* Секция 7. Футер формы */}
        <div className="lg:col-span-2 sticky bottom-16 lg:bottom-4 z-30">
          <div className="bg-surface rounded-card shadow-modal border border-brand-100/60 p-3 flex flex-col sm:flex-row items-stretch sm:items-center gap-2">
            <button
              type="button"
              onClick={() => navigate(-1)}
              className="h-11 px-5 rounded-xl text-sm font-semibold text-brand-600 hover:bg-brand-50 transition-colors"
            >
              Отмена
            </button>
            <div className="flex-1" />
            <button
              type="button"
              disabled={create.isPending}
              onClick={handleSubmit((v) => doSubmit(v, true), onInvalid)}
              className="h-11 px-5 rounded-xl bg-surface border border-brand-100 text-ink-900 text-sm font-semibold hover:bg-brand-50 transition-colors disabled:opacity-50"
            >
              Сохранить и создать ещё
            </button>
            <button
              type="submit"
              disabled={create.isPending || justCreated}
              className={cn(
                'inline-flex items-center justify-center gap-2 h-11 px-6 rounded-xl text-white text-sm font-semibold transition-all duration-150 active:scale-[0.97] disabled:opacity-70',
                justCreated ? 'bg-success' : 'bg-accent hover:bg-accent-hover hover:-translate-y-px'
              )}
            >
              {create.isPending ? (
                <Loader2 size={16} className="animate-spin" />
              ) : justCreated ? (
                <Check size={16} strokeWidth={2.5} />
              ) : null}
              {create.isPending ? 'Создаём…' : justCreated ? 'Создано' : 'Создать инструмент'}
            </button>
          </div>
        </div>
      </form>

      <Toast text={toast} />
    </div>
  )
}
