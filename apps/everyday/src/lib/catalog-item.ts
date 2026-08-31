export type CatalogTool = {
  id: string
  numericId: number
  vn: string
  name: string
  photo: string
  categoryId: number | null
  categoryName: string
  brandId: number | null
  statusId: number | null
  status: string
  statusName: string
  statusColor: string
  statusBg: string
  siteId: number | null
  siteName: string | null
  warehouseId: number | null
  warehouseName: string | null
  assigneeId: number | null
  assigneeName: string | null
  assigneePhone: string | null
  assigneeAvatar: string | null
  isMaterial: boolean
  quantity?: number | null
  unit?: string | null
  hasQr: boolean
  qrCode: string | null
  price: number
  serial?: string | null
  createdAt: string
  dueAt?: string | null
  stockQty?: number | null
  issuedQty?: number | null
  totalQty?: number | null
  holders?: Array<{ userId?: number; quantity?: number; user?: Person | null; internalId?: string }>
}

type Photo = { url: string; isTitle: boolean }
type Named = { id: number; name: string }
type Status = { id: number; name: string; slug: string; color: string; bg: string }
type Person = { id: number; fullName: string; phone: string; avatarUrl: string | null }

export function mapItemToCatalogTool(row: {
  id: number
  internalId: string
  title: string
  categoryId: number | null
  brandId: number | null
  statusId: number | null
  buildingSiteId: number | null
  storageId: number | null
  responsibleUserId: number | null
  quantitative: boolean
  quantity: number | null
  unit: string | null
  qrCode: string | null
  cost: number | null
  serialNumber: string | null
  createdAt: Date | string
  dueAt?: string | null
  photos?: Photo[]
  category?: Named | null
  brand?: Named | null
  status?: Status | null
  buildingSite?: Named | null
  storage?: Named | null
  responsible?: Person | null
  stockQty?: number | null
  issuedQty?: number | null
  totalQty?: number | null
  holders?: Array<{ userId?: number; quantity?: number; user?: Person | null; internalId?: string }>
}): CatalogTool {
  const photo = row.photos?.find((p) => p.isTitle)?.url ?? row.photos?.[0]?.url ?? "/empty-catalog.svg"
  const createdAt = row.createdAt instanceof Date ? row.createdAt.toISOString() : String(row.createdAt)
  return {
    id: String(row.id),
    numericId: row.id,
    vn: row.internalId,
    name: row.title,
    photo,
    categoryId: row.categoryId,
    categoryName: row.category?.name ?? "—",
    brandId: row.brandId,
    statusId: row.statusId,
    status: row.status?.slug ?? "in-stock",
    statusName: row.status?.name ?? "—",
    statusColor: row.status?.color ?? "#5E629B",
    statusBg: row.status?.bg ?? "#EDEDF7",
    siteId: row.buildingSiteId,
    siteName: row.buildingSite?.name ?? null,
    warehouseId: row.storageId,
    warehouseName: row.storage?.name ?? null,
    assigneeId: row.responsibleUserId,
    assigneeName: row.responsible?.fullName ?? null,
    assigneePhone: row.responsible?.phone ?? null,
    assigneeAvatar: row.responsible?.avatarUrl ?? null,
    isMaterial: row.quantitative,
    quantity: row.quantity,
    unit: row.unit,
    hasQr: Boolean(row.qrCode),
    qrCode: row.qrCode,
    price: row.cost ?? 0,
    serial: row.serialNumber,
    createdAt,
    dueAt: row.dueAt ?? null,
    stockQty: row.stockQty ?? null,
    issuedQty: row.issuedQty ?? null,
    totalQty: row.totalQty ?? null,
    holders: row.holders,
  }
}
