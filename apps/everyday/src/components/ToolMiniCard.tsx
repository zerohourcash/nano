import { useNavigate } from 'react-router'
import { Building2, Phone, Warehouse } from 'lucide-react'
import { motion } from 'framer-motion'
import { cn } from '@/lib/utils'
import type { CatalogTool } from '@/lib/catalog-item'
import { QrBadge, MaterialBadge } from '@/components/StatusBadge'
import { useStore } from '@/lib/store'

interface Props {
  tool: CatalogTool
  selectionMode?: boolean
  onCallClick?: (tool: CatalogTool) => void
}

export default function ToolMiniCard({ tool, selectionMode = false, onCallClick }: Props) {
  const navigate = useNavigate()
  const { selectedToolIds, toggleToolSelected, setSelectionMode } = useStore()
  const selected = selectedToolIds.has(tool.id)

  const openCard = () => navigate(`/tool/${tool.numericId}`)

  const onCheck = (e: React.MouseEvent) => {
    e.stopPropagation()
    if (!selectionMode && !selected) setSelectionMode(true)
    toggleToolSelected(tool.id)
  }

  return (
    <motion.article
      onClick={openCard}
      whileHover={{ y: -2 }}
      transition={{ duration: 0.2, ease: 'easeOut' }}
      className={cn(
        'group relative bg-surface rounded-mini border shadow-card p-3 cursor-pointer transition-shadow hover:shadow-hover',
        selected ? 'border-brand-600 ring-2 ring-brand-600/20' : 'border-brand-100/60',
      )}
    >
      <div className="relative overflow-hidden rounded-[10px] aspect-[4/3] bg-brand-50">
        <img
          src={tool.photo}
          alt={tool.name}
          loading="lazy"
          className="w-full h-full object-cover transition-transform duration-300 group-hover:scale-[1.03]"
        />
        <button
          onClick={onCheck}
          aria-label={selected ? 'Снять выбор' : 'Выбрать'}
          className={cn(
            'absolute left-2 top-2 w-5 h-5 rounded-md border-[1.5px] flex items-center justify-center transition-all duration-150',
            'bg-white/80 backdrop-blur-sm',
            selected
              ? 'bg-brand-600 border-brand-600 opacity-100'
              : 'border-brand-100 opacity-0 group-hover:opacity-100',
            (selectionMode || selected) && 'opacity-100',
          )}
        >
          {selected && (
            <svg width="12" height="12" viewBox="0 0 12 12" fill="none">
              <path d="M2 6.2L4.8 9L10 3" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          )}
        </button>
        {(tool.hasQr || tool.isMaterial) && (
          <div className="absolute right-2 bottom-2 flex gap-1">
            {tool.hasQr && <QrBadge />}
            {tool.isMaterial && <MaterialBadge />}
          </div>
        )}
      </div>

      <div className="pt-2.5 space-y-1.5">
        <div className="flex items-center justify-between gap-2">
          <span className="font-mono-num text-ink-500">{tool.vn}</span>
          <span className="inline-flex items-center gap-1.5 text-xs font-semibold" style={{ color: tool.statusColor }}>
            <span className="w-1.5 h-1.5 rounded-full shrink-0" style={{ background: tool.statusColor }} />
            {tool.statusName}
          </span>
        </div>
        <h3 className="text-[15px] leading-[22px] font-semibold text-ink-900 line-clamp-2 min-h-[44px]">{tool.name}</h3>

        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-ink-500">
          {tool.siteName && (
            <span className="inline-flex items-center gap-1 min-w-0">
              <Building2 size={13} strokeWidth={1.75} className="shrink-0 text-ink-300" />
              <span className="truncate">{tool.siteName}</span>
            </span>
          )}
          {tool.warehouseName && (
            <span className="inline-flex items-center gap-1 min-w-0">
              <Warehouse size={13} strokeWidth={1.75} className="shrink-0 text-ink-300" />
              <span className="truncate">{tool.warehouseName}</span>
            </span>
          )}
          {(tool.totalQty ?? 0) > 0 && (
            <span className="font-mono-num text-ink-500">
              на складе {tool.stockQty ?? 0}
              {(tool.issuedQty ?? 0) > 0 ? ` · выдано ${tool.issuedQty}` : ''}
              {tool.isMaterial && tool.unit ? ` ${tool.unit}` : ' шт'}
            </span>
          )}
        </div>

        <div className="flex items-center gap-2 pt-1 border-t border-brand-100/60">
          {tool.holders && tool.holders.length > 0 ? (
            <div className="min-w-0 flex-1">
              <div className="text-[11px] text-ink-500">Сейчас у</div>
              <div className="text-xs font-semibold text-ink-900 truncate">
                {tool.holders
                  .map((h) => {
                    const name = h.user?.fullName ?? 'сотрудник'
                    const q = h.quantity && h.quantity !== 1 ? ` ×${h.quantity}` : ''
                    const vn = h.internalId ? ` (${h.internalId})` : ''
                    return name + q + vn
                  })
                  .join(', ')}
              </div>
            </div>
          ) : tool.assigneeName ? (
            <>
              {tool.assigneeAvatar ? (
                <img
                  src={tool.assigneeAvatar}
                  alt={tool.assigneeName}
                  className="w-5 h-5 rounded-full object-cover border border-brand-100"
                />
              ) : (
                <span className="w-5 h-5 rounded-full bg-brand-100" />
              )}
              <span className="text-xs font-semibold text-ink-900 truncate flex-1">{tool.assigneeName}</span>
              {tool.assigneePhone && (
                <button
                  onClick={(e) => {
                    e.stopPropagation()
                    onCallClick?.(tool)
                    window.location.href = `tel:${tool.assigneePhone!.replace(/[^+\d]/g, '')}`
                  }}
                  aria-label={`Позвонить: ${tool.assigneeName}`}
                  className="w-8 h-8 shrink-0 flex items-center justify-center rounded-lg text-teal-dark hover:bg-teal/15 transition-colors"
                >
                  <Phone size={15} strokeWidth={1.75} />
                </button>
              )}
            </>
          ) : (
            <span className="text-xs text-ink-300 py-1.5">На складе</span>
          )}
        </div>
      </div>
    </motion.article>
  )
}
