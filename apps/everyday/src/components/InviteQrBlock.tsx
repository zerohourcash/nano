import { QRCodeCanvas } from 'qrcode.react'

export default function InviteQrBlock({ value, size = 240 }: { value: string; size?: number }) {
  if (!value) return null
  return (
    <div
      className="inline-flex items-center justify-center rounded-2xl border border-brand-100 bg-white p-4 shadow-card"
      style={{ minWidth: size + 32, minHeight: size + 32 }}
    >
      <QRCodeCanvas
        value={value}
        size={size}
        level="M"
        includeMargin
        bgColor="#ffffff"
        fgColor="#1B1E3D"
        style={{ width: size, height: size, display: 'block' }}
      />
    </div>
  )
}
