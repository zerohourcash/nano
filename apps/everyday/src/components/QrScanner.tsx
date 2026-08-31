import { useEffect, useRef, useState } from 'react'
import { Camera, CameraOff, ImagePlus, Keyboard } from 'lucide-react'
import { cn } from '@/lib/utils'

interface Props {
  onCode: (code: string) => void
  className?: string
}

type NativeBridge = {
  scanQr?: () => void
}

function nativeBridge(): NativeBridge | null {
  if (typeof window === 'undefined') return null
  const w = window as unknown as { MeshKeeperNative?: NativeBridge }
  return w.MeshKeeperNative ?? null
}

async function openCamera(): Promise<MediaStream> {
  const media = navigator.mediaDevices
  if (!media?.getUserMedia) {
    throw new Error('no-media')
  }
  const attempts: MediaStreamConstraints[] = [
    { video: { facingMode: { ideal: 'environment' } }, audio: false },
    { video: { facingMode: 'environment' }, audio: false },
    { video: true, audio: false },
  ]
  let last: unknown
  for (const constraints of attempts) {
    try {
      return await media.getUserMedia(constraints)
    } catch (e) {
      last = e
    }
  }
  throw last instanceof Error ? last : new Error('camera')
}

export default function QrScanner({ onCode, className }: Props) {
  const videoRef = useRef<HTMLVideoElement>(null)
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const fileRef = useRef<HTMLInputElement>(null)
  const onCodeRef = useRef(onCode)
  useEffect(() => {
    onCodeRef.current = onCode
  }, [onCode])
  const lastRef = useRef('')
  const [error, setError] = useState<string | null>(null)
  const [manual, setManual] = useState('')
  const native = nativeBridge()
  const hasNative = typeof native?.scanQr === 'function'

  // Дедуп нужен только чтобы один код не сработал 60 раз в секунду.
  // Через RESCAN_MS ту же бирку можно отсканировать снова.
  const RESCAN_MS = 2500
  const lastAtRef = useRef(0)
  const emit = (raw: string) => {
    const value = raw.trim()
    if (!value) return
    const now = Date.now()
    if (value === lastRef.current && now - lastAtRef.current < RESCAN_MS) return
    lastRef.current = value
    lastAtRef.current = now
    onCodeRef.current(value)
  }

  useEffect(() => {
    const w = window as unknown as { __onNativeQr?: (code: string) => void }
    w.__onNativeQr = (code: string) => emit(code)
    if (hasNative) {
      try {
        native?.scanQr?.()
      } catch {
        /* user can tap the button */
      }
      return () => {
        delete w.__onNativeQr
      }
    }

    let stream: MediaStream | null = null
    let raf = 0
    let detector: { detect: (source: HTMLVideoElement) => Promise<Array<{ rawValue?: string }>> } | null = null
    let stopped = false

    const tick = async () => {
      const video = videoRef.current
      if (!video || video.readyState < 2) {
        if (!stopped) raf = requestAnimationFrame(() => void tick())
        return
      }
      try {
        if (detector) {
          const codes = await detector.detect(video)
          if (codes[0]?.rawValue) emit(codes[0].rawValue)
        } else {
          const canvas = canvasRef.current
          const ctx = canvas?.getContext('2d')
          if (canvas && ctx && video.videoWidth > 0) {
            canvas.width = video.videoWidth
            canvas.height = video.videoHeight
            ctx.drawImage(video, 0, 0)
            const { default: jsQR } = await import('jsqr')
            const image = ctx.getImageData(0, 0, canvas.width, canvas.height)
            const result = jsQR(image.data, image.width, image.height)
            if (result?.data) emit(result.data)
          }
        }
      } catch {
        /* keep scanning */
      }
      if (!stopped) raf = requestAnimationFrame(() => void tick())
    }

    const start = async () => {
      try {
        const Detector = (window as unknown as { BarcodeDetector?: new (opts: { formats: string[] }) => typeof detector }).BarcodeDetector
        if (Detector) detector = new Detector({ formats: ['qr_code'] })
        stream = await openCamera()
        if (videoRef.current) {
          videoRef.current.srcObject = stream
          await videoRef.current.play()
        }
        setError(null)
        raf = requestAnimationFrame(() => void tick())
      } catch {
        setError('Камера недоступна. Разрешите доступ или загрузите фото QR.')
      }
    }

    void start()
    return () => {
      stopped = true
      cancelAnimationFrame(raf)
      stream?.getTracks().forEach((t) => t.stop())
      delete w.__onNativeQr
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasNative])

  const decodeFile = async (file: File) => {
    try {
      const bmp = await createImageBitmap(file)
      const canvas = document.createElement('canvas')
      canvas.width = bmp.width
      canvas.height = bmp.height
      const ctx = canvas.getContext('2d')
      if (!ctx) return
      ctx.drawImage(bmp, 0, 0)
      const { default: jsQR } = await import('jsqr')
      const image = ctx.getImageData(0, 0, canvas.width, canvas.height)
      const result = jsQR(image.data, image.width, image.height)
      if (result?.data) emit(result.data)
      else setError('На фото нет QR-кода. Снимите код ближе и ровнее.')
    } catch {
      setError('Не удалось прочитать фото')
    }
  }

  return (
    <div className={cn('space-y-3', className)}>
      {hasNative ? (
        <div className="flex flex-col items-center gap-3 rounded-card border border-brand-100 bg-white px-5 py-8 text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-brand-50 text-brand-600">
            <Camera size={28} />
          </div>
          <p className="text-sm font-semibold text-ink-900">Наведите камеру на QR группы</p>
          <p className="text-[13px] text-ink-500">Откроется системная камера телефона — так она работает стабильнее, чем в браузере.</p>
          <button
            type="button"
            onClick={() => native?.scanQr?.()}
            className="mt-1 inline-flex h-12 items-center justify-center gap-2 rounded-xl bg-accent px-6 text-sm font-semibold text-white hover:bg-accent-hover"
          >
            <Camera size={18} />
            Открыть камеру
          </button>
        </div>
      ) : (
        <div className="relative overflow-hidden rounded-card bg-ink-900 aspect-[3/4] max-h-[420px]">
          <video ref={videoRef} className="h-full w-full object-cover" playsInline muted autoPlay />
          <canvas ref={canvasRef} className="hidden" />
          <div className="pointer-events-none absolute inset-12 rounded-2xl border-2 border-white/80 shadow-[0_0_0_9999px_rgba(0,0,0,.35)]" />
          {error && (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-ink-900/80 px-6 text-center text-white">
              <CameraOff size={28} />
              <p className="text-sm font-semibold">{error}</p>
            </div>
          )}
        </div>
      )}

      <input
        ref={fileRef}
        type="file"
        accept="image/*"
        capture="environment"
        className="hidden"
        onChange={(e) => {
          const file = e.target.files?.[0]
          if (file) void decodeFile(file)
          e.target.value = ''
        }}
      />
      <button
        type="button"
        onClick={() => fileRef.current?.click()}
        className="flex h-11 w-full items-center justify-center gap-2 rounded-xl border border-brand-100 bg-white text-sm font-semibold text-ink-900 hover:bg-brand-50"
      >
        <ImagePlus size={16} />
        Загрузить фото QR
      </button>

      <form
        className="flex gap-2"
        onSubmit={(e) => {
          e.preventDefault()
          const code = manual.trim()
          if (code) emit(code)
        }}
      >
        <div className="relative flex-1">
          <Keyboard size={16} className="absolute left-3 top-1/2 -translate-y-1/2 text-ink-300" />
          <input
            value={manual}
            onChange={(e) => setManual(e.target.value)}
            placeholder="Или вставьте ссылку / токен"
            className="h-11 w-full rounded-xl border border-brand-100 bg-white pl-9 pr-3 text-sm"
          />
        </div>
        <button
          type="submit"
          className="h-11 rounded-xl bg-accent px-4 text-sm font-semibold text-white hover:bg-accent-hover"
        >
          Далее
        </button>
      </form>
    </div>
  )
}
