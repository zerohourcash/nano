// Единый источник демо-данных MeshKeeper (design.md §12).
// Все страницы импортируют отсюда: `import { tools, users, ... } from '@/lib/mock-data'`.

// ─── Типы ────────────────────────────────────────────────────────────────────

export interface Workspace {
  id: string
  name: string
  prefix: string
}

export type UserRole =
  | 'Кладовщик'
  | 'Прораб'
  | 'Мастер'
  | 'Руководитель'
  | 'Монтажник'
  | 'Бухгалтер'

export interface User {
  id: string
  name: string
  role: UserRole
  phone: string
  avatar: string
}

export interface Warehouse {
  id: string
  name: string
  address: string
}

export interface Site {
  id: string
  name: string
}

export interface Category {
  id: string
  name: string
}

export interface Brand {
  id: string
  name: string
}

export type ToolStatus = 'in-work' | 'in-repair' | 'in-stock' | 'written-off'

export interface StatusInfo {
  id: ToolStatus
  name: string
  /** цвет точки/текста */
  color: string
  /** цвет фона бейджа */
  bg: string
}

export interface Tool {
  id: string
  /** внутренний номер, напр. ВН-0142 */
  vn: string
  name: string
  photo: string
  categoryId: string
  brandId: string
  status: ToolStatus
  siteId: string | null
  warehouseId: string
  /** ответственный, null — никто */
  assigneeId: string | null
  /** количественный учёт (материалы) */
  isMaterial: boolean
  quantity?: number
  unit?: string
  hasQr: boolean
  price: number
  serial?: string
  createdAt: string
}

export type TransferStatus = 'pending-send' | 'pending-receive' | 'accepted' | 'rejected'

export interface Transfer {
  id: string
  /** ID передачи, напр. ПП-0042 */
  code: string
  toolIds: string[]
  fromUserId: string
  toUserId: string
  siteId: string | null
  warehouseId: string
  status: TransferStatus
  createdAt: string
  comment?: string
}

export type HistoryAction =
  | 'create'
  | 'transfer-send'
  | 'transfer-receive'
  | 'status-change'
  | 'write-off'
  | 'restock'
  | 'edit'

export interface HistoryEvent {
  id: string
  toolId: string
  action: HistoryAction
  userId: string
  date: string
  description: string
}

export type NotificationType = 'transfer' | 'reminder' | 'inventory' | 'system'

export interface AppNotification {
  id: string
  type: NotificationType
  title: string
  text: string
  date: string
  read: boolean
}

// ─── Рабочие пространства ────────────────────────────────────────────────────

export const workspaces: Workspace[] = [
  { id: 'ws-1', name: 'ООО «СтройМонтаж»', prefix: 'ВН-' },
  { id: 'ws-2', name: 'ИП «РемСервис»', prefix: 'РС-' },
]

// ─── Пользователи ────────────────────────────────────────────────────────────

export const users: User[] = [
  { id: 'u-1', name: 'Алексей Кузнецов', role: 'Кладовщик', phone: '+7 921 555-01-42', avatar: '/avatar-1.png' },
  { id: 'u-2', name: 'Марина Орлова', role: 'Прораб', phone: '+7 921 555-02-17', avatar: '/avatar-2.png' },
  { id: 'u-3', name: 'Игорь Савельев', role: 'Мастер', phone: '+7 921 555-03-88', avatar: '/avatar-3.png' },
  { id: 'u-4', name: 'Ольга Демидова', role: 'Руководитель', phone: '+7 921 555-04-29', avatar: '/avatar-4.png' },
  { id: 'u-5', name: 'Павел Ким', role: 'Монтажник', phone: '+7 921 555-05-63', avatar: '/avatar-5.png' },
  { id: 'u-6', name: 'Елена Ветрова', role: 'Бухгалтер', phone: '+7 921 555-06-91', avatar: '/avatar-6.png' },
]

/** Текущий пользователь (после входа) */
export const currentUser: User = users[0]

// ─── Склады / объекты / справочники ─────────────────────────────────────────

export const warehouses: Warehouse[] = [
  { id: 'wh-1', name: 'Центральный склад', address: 'СПб, Индустриальный пр. 44' },
  { id: 'wh-2', name: 'Склад №2', address: 'Пушкин' },
]

export const sites: Site[] = [
  { id: 'site-1', name: 'ЖК «Северная звезда»' },
  { id: 'site-2', name: 'БЦ «Лиговский 87»' },
  { id: 'site-3', name: 'ТРЦ «Галерея»' },
]

export const categories: Category[] = [
  { id: 'cat-1', name: 'Электроинструмент' },
  { id: 'cat-2', name: 'Измерительный и контрольный инструмент' },
  { id: 'cat-3', name: 'Оргтехника и компьютеры' },
  { id: 'cat-4', name: 'Ручной инструмент' },
  { id: 'cat-5', name: 'Расходные материалы' },
]

export const brands: Brand[] = [
  { id: 'br-1', name: 'Bosch' },
  { id: 'br-2', name: 'Makita' },
  { id: 'br-3', name: 'DeWalt' },
  { id: 'br-4', name: 'Karcher' },
  { id: 'br-5', name: 'Metabo' },
  { id: 'br-6', name: 'Зубр' },
]

export const statuses: StatusInfo[] = [
  { id: 'in-work', name: 'В работе', color: '#2E9E5B', bg: '#C8FCD2' },
  { id: 'in-repair', name: 'В ремонте', color: '#A87C0F', bg: '#FBFCC8' },
  { id: 'in-stock', name: 'На складе', color: '#5E629B', bg: '#EDEDF7' },
  { id: 'written-off', name: 'Списан', color: '#D64545', bg: '#FAD8D1' },
]

export const statusById = (id: ToolStatus): StatusInfo =>
  statuses.find((s) => s.id === id) ?? statuses[0]

// ─── Инструменты ─────────────────────────────────────────────────────────────

const baseTools: Tool[] = [
  {
    id: 't-0142', vn: 'ВН-0142', name: 'Перфоратор Bosch GBH 8-45 DV',
    photo: '/tool-bosch-gbh.png', categoryId: 'cat-1', brandId: 'br-1',
    status: 'in-work', siteId: 'site-1', warehouseId: 'wh-1', assigneeId: 'u-2',
    isMaterial: false, hasQr: true, price: 68400, serial: 'GBH-8845-2210', createdAt: '2024-03-12',
  },
  {
    id: 't-0087', vn: 'ВН-0087', name: 'Шуруповёрт аккумуляторный Makita DF333',
    photo: '/tool-makita-df.png', categoryId: 'cat-1', brandId: 'br-2',
    status: 'in-work', siteId: 'site-1', warehouseId: 'wh-1', assigneeId: 'u-3',
    isMaterial: false, hasQr: true, price: 18900, serial: 'MK-DF333-7741', createdAt: '2023-11-02',
  },
  {
    id: 't-0201', vn: 'ВН-0201', name: 'Мойка высокого давления Karcher K5',
    photo: '/tool-karcher-k5.png', categoryId: 'cat-1', brandId: 'br-4',
    status: 'in-stock', siteId: null, warehouseId: 'wh-1', assigneeId: null,
    isMaterial: false, hasQr: true, price: 32500, serial: 'KR-K5-0093', createdAt: '2024-06-21',
  },
  {
    id: 't-0115', vn: 'ВН-0115', name: 'Торцовочная пила DeWalt DWS780',
    photo: '/tool-dewalt-dws.png', categoryId: 'cat-1', brandId: 'br-3',
    status: 'in-repair', siteId: null, warehouseId: 'wh-2', assigneeId: null,
    isMaterial: false, hasQr: false, price: 89700, serial: 'DW-780-5124', createdAt: '2023-08-14',
  },
  {
    id: 't-0156', vn: 'ВН-0156', name: 'Ноутбук MSI Modern 15',
    photo: '/tool-msi-laptop.png', categoryId: 'cat-3', brandId: 'br-1',
    status: 'in-work', siteId: 'site-2', warehouseId: 'wh-1', assigneeId: 'u-4',
    isMaterial: false, hasQr: true, price: 74900, serial: 'MSI-15-9921', createdAt: '2024-01-30',
  },
  {
    id: 't-0063', vn: 'ВН-0063', name: 'Лазерный уровень Зубр ЛУ-360',
    photo: '/tool-laser-level.png', categoryId: 'cat-2', brandId: 'br-6',
    status: 'in-stock', siteId: null, warehouseId: 'wh-2', assigneeId: null,
    isMaterial: false, hasQr: true, price: 12700, serial: 'ZB-360-3355', createdAt: '2023-05-19',
  },
  {
    id: 't-0178', vn: 'ВН-0178', name: 'Углошлифмашина Metabo W 650',
    photo: '/tool-metabo-grinder.png', categoryId: 'cat-1', brandId: 'br-5',
    status: 'in-work', siteId: 'site-3', warehouseId: 'wh-1', assigneeId: 'u-5',
    isMaterial: false, hasQr: false, price: 9800, serial: 'MT-W650-1187', createdAt: '2024-04-08',
  },
  {
    id: 't-0231', vn: 'ВН-0231', name: 'Тряпки ветошь 30×30, упаковка 100 шт',
    photo: '/tool-rags.png', categoryId: 'cat-5', brandId: 'br-6',
    status: 'in-stock', siteId: null, warehouseId: 'wh-1', assigneeId: null,
    isMaterial: true, quantity: 96, unit: 'шт', hasQr: false, price: 1450, createdAt: '2024-09-03',
  },
]

// Расширенный список для демонстрации ленивой подгрузки (8 базовых → 20 карточек)
const extraTools: Tool[] = [
  { ...baseTools[1], id: 't-0088', vn: 'ВН-0088', status: 'in-stock', siteId: null, assigneeId: null, createdAt: '2024-02-11' },
  { ...baseTools[0], id: 't-0143', vn: 'ВН-0143', status: 'in-repair', siteId: null, warehouseId: 'wh-2', assigneeId: null, createdAt: '2024-05-27' },
  { ...baseTools[6], id: 't-0179', vn: 'ВН-0179', siteId: 'site-1', assigneeId: 'u-2', hasQr: true, createdAt: '2024-04-22' },
  { ...baseTools[2], id: 't-0202', vn: 'ВН-0202', status: 'in-work', siteId: 'site-3', assigneeId: 'u-5', createdAt: '2024-07-15' },
  { ...baseTools[5], id: 't-0064', vn: 'ВН-0064', status: 'in-work', siteId: 'site-2', assigneeId: 'u-3', createdAt: '2023-06-30' },
  { ...baseTools[4], id: 't-0157', vn: 'ВН-0157', status: 'in-stock', siteId: null, assigneeId: null, createdAt: '2024-03-19' },
  { ...baseTools[7], id: 't-0232', vn: 'ВН-0232', quantity: 54, warehouseId: 'wh-2', createdAt: '2024-09-10' },
  { ...baseTools[3], id: 't-0116', vn: 'ВН-0116', status: 'in-work', siteId: 'site-1', assigneeId: 'u-5', hasQr: true, createdAt: '2023-10-05' },
  { ...baseTools[1], id: 't-0089', vn: 'ВН-0089', status: 'written-off', siteId: null, warehouseId: 'wh-1', assigneeId: null, createdAt: '2022-12-01' },
  { ...baseTools[0], id: 't-0144', vn: 'ВН-0144', status: 'in-work', siteId: 'site-3', assigneeId: 'u-3', createdAt: '2024-08-02' },
  { ...baseTools[6], id: 't-0180', vn: 'ВН-0180', status: 'in-stock', siteId: null, assigneeId: null, createdAt: '2024-06-11' },
  { ...baseTools[2], id: 't-0203', vn: 'ВН-0203', status: 'in-stock', siteId: null, warehouseId: 'wh-2', createdAt: '2024-10-01' },
]

export const tools: Tool[] = [...baseTools, ...extraTools]

/** Общий счётчик каталога (демо-цифра из catalog.md) */
export const TOTAL_TOOLS_COUNT = 142

// ─── Передачи ────────────────────────────────────────────────────────────────

export const transfers: Transfer[] = [
  { id: 'tr-1', code: 'ПП-0042', toolIds: ['t-0142'], fromUserId: 'u-1', toUserId: 'u-2', siteId: 'site-1', warehouseId: 'wh-1', status: 'pending-send', createdAt: '2025-08-18', comment: 'Для корпуса 3' },
  { id: 'tr-2', code: 'ПП-0043', toolIds: ['t-0178'], fromUserId: 'u-1', toUserId: 'u-5', siteId: 'site-3', warehouseId: 'wh-1', status: 'pending-send', createdAt: '2025-08-19' },
  { id: 'tr-3', code: 'ПП-0044', toolIds: ['t-0088'], fromUserId: 'u-1', toUserId: 'u-3', siteId: 'site-2', warehouseId: 'wh-1', status: 'pending-send', createdAt: '2025-08-20' },
  { id: 'tr-4', code: 'ПП-0039', toolIds: ['t-0063'], fromUserId: 'u-3', toUserId: 'u-1', siteId: null, warehouseId: 'wh-2', status: 'pending-receive', createdAt: '2025-08-17' },
  { id: 'tr-5', code: 'ПП-0040', toolIds: ['t-0156'], fromUserId: 'u-4', toUserId: 'u-1', siteId: null, warehouseId: 'wh-1', status: 'pending-receive', createdAt: '2025-08-18' },
  { id: 'tr-6', code: 'ПП-0035', toolIds: ['t-0201'], fromUserId: 'u-1', toUserId: 'u-2', siteId: 'site-1', warehouseId: 'wh-1', status: 'accepted', createdAt: '2025-08-10' },
  { id: 'tr-7', code: 'ПП-0031', toolIds: ['t-0115'], fromUserId: 'u-5', toUserId: 'u-1', siteId: null, warehouseId: 'wh-2', status: 'rejected', createdAt: '2025-08-05', comment: 'Повреждён диск' },
]

// ─── История (журнал операций — будущий леджер) ─────────────────────────────

export const history: HistoryEvent[] = [
  { id: 'h-1', toolId: 't-0142', action: 'transfer-send', userId: 'u-1', date: '2025-08-18T09:24:00', description: 'Передача Марине Орловой (ПП-0042)' },
  { id: 'h-2', toolId: 't-0201', action: 'transfer-receive', userId: 'u-2', date: '2025-08-10T15:02:00', description: 'Приём от Алексея Кузнецова (ПП-0035)' },
  { id: 'h-3', toolId: 't-0115', action: 'status-change', userId: 'u-1', date: '2025-08-06T11:40:00', description: 'Статус изменён: В работе → В ремонте' },
  { id: 'h-4', toolId: 't-0231', action: 'restock', userId: 'u-1', date: '2025-08-04T08:15:00', description: 'Пополнение: +20 шт (закупка)' },
  { id: 'h-5', toolId: 't-0089', action: 'write-off', userId: 'u-4', date: '2025-07-29T16:55:00', description: 'Списание: физический износ' },
  { id: 'h-6', toolId: 't-0203', action: 'create', userId: 'u-1', date: '2024-10-01T10:00:00', description: 'Инструмент добавлен в каталог' },
  { id: 'h-7', toolId: 't-0156', action: 'edit', userId: 'u-4', date: '2025-07-20T13:30:00', description: 'Обновлены документы и фото' },
  { id: 'h-8', toolId: 't-0178', action: 'transfer-send', userId: 'u-1', date: '2025-08-19T12:05:00', description: 'Передача Павлу Киму (ПП-0043)' },
]

// ─── Уведомления ─────────────────────────────────────────────────────────────

export const notifications: AppNotification[] = [
  { id: 'n-1', type: 'transfer', title: 'Ожидает приёма', text: 'Передача ПП-0039: Лазерный уровень Зубр от Игоря Савельева', date: '2025-08-17T10:12:00', read: false },
  { id: 'n-2', type: 'transfer', title: 'Ожидает приёма', text: 'Передача ПП-0040: Ноутбук MSI от Ольги Демидовой', date: '2025-08-18T09:03:00', read: false },
  { id: 'n-3', type: 'reminder', title: 'Напоминание о поверке', text: 'Лазерный уровень Зубр ЛУ-360 — поверка до 01.09.2025', date: '2025-08-16T08:00:00', read: false },
  { id: 'n-4', type: 'inventory', title: 'Инвентаризация запланирована', text: 'Центральный склад — сверка начнётся 25.08.2025', date: '2025-08-15T14:45:00', read: true },
  { id: 'n-5', type: 'system', title: 'Синхронизация завершена', text: 'Журнал операций сохранён на сервере', date: '2025-08-14T18:20:00', read: true },
]

// ─── Хелперы ─────────────────────────────────────────────────────────────────

export const userById = (id: string | null | undefined): User | undefined =>
  users.find((u) => u.id === id)

export const warehouseById = (id: string | null | undefined): Warehouse | undefined =>
  warehouses.find((w) => w.id === id)

export const siteById = (id: string | null | undefined): Site | undefined =>
  sites.find((s) => s.id === id)

export const categoryById = (id: string): Category | undefined =>
  categories.find((c) => c.id === id)

export const brandById = (id: string): Brand | undefined =>
  brands.find((b) => b.id === id)

export const toolById = (id: string): Tool | undefined =>
  tools.find((t) => t.id === id)
