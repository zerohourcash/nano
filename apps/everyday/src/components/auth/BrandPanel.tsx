import { useRef } from 'react'
import gsap from 'gsap'
import { SplitText } from 'gsap/SplitText'
import { useGSAP } from '@gsap/react'
import { QrCode, ArrowLeftRight, Network } from 'lucide-react'
import MeshCanvas from '@/components/auth/MeshCanvas'

gsap.registerPlugin(SplitText, useGSAP)

const FEATURES = [
  { icon: QrCode, text: 'QR-учёт каждой единицы: от перфоратора до пачки ветоши' },
  { icon: ArrowLeftRight, text: 'Приём-передача с фотофиксацией за 10 секунд' },
  { icon: Network, text: 'Подписанный журнал хранится локально; внешний sync выключен по умолчанию' },
]

/**
 * Левая бренд-панель экрана входа (auth.md, Секция 1).
 * GSAP-зона: входные анимации (SplitText по словам) + параллакс контента
 * против движения курсора (max 8px, quickTo). Framer Motion здесь не используется.
 */
export default function BrandPanel() {
  const rootRef = useRef<HTMLDivElement>(null)
  const contentRef = useRef<HTMLDivElement>(null)

  useGSAP(
    () => {
      const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches
      const split = new SplitText('.auth-display', { type: 'words' })

      if (reduced) {
        // prefers-reduced-motion: только fade, без stagger/parallax
        gsap.fromTo(
          ['.auth-logo', '.auth-display', '.auth-subtitle', '.auth-feature', '.auth-footer'],
          { opacity: 0 },
          { opacity: 1, duration: 0.3, stagger: 0.05, ease: 'power1.out' },
        )
        return () => split.revert()
      }

      const tl = gsap.timeline({ defaults: { ease: 'power3.out' } })
      // Логотип: fade + slide down 20px, задержка 0.1s
      tl.fromTo(
        '.auth-logo',
        { opacity: 0, y: -20 },
        { opacity: 1, y: 0, duration: 0.5 },
        0.1,
      )
      // Заголовок: SplitText по словам, stagger 0.08s, slide up 30px + rotateX 20°→0
      tl.fromTo(
        split.words,
        { opacity: 0, y: 30, rotateX: 20, transformPerspective: 600 },
        { opacity: 1, y: 0, rotateX: 0, duration: 0.6, stagger: 0.08 },
        0.25,
      )
      tl.fromTo(
        '.auth-subtitle',
        { opacity: 0, y: 16 },
        { opacity: 1, y: 0, duration: 0.5 },
        0.7,
      )
      // Фичи: stagger 0.12s, slide left 24px, начиная с 0.6s
      tl.fromTo(
        '.auth-feature',
        { opacity: 0, x: -24 },
        { opacity: 1, x: 0, duration: 0.45, stagger: 0.12 },
        0.6,
      )
      tl.fromTo('.auth-footer', { opacity: 0 }, { opacity: 1, duration: 0.4 }, 1.2)

      // Параллакс: контент лёгко смещается против движения курсора (max 8px)
      const xTo = gsap.quickTo(contentRef.current, 'x', { duration: 0.6, ease: 'power2.out' })
      const yTo = gsap.quickTo(contentRef.current, 'y', { duration: 0.6, ease: 'power2.out' })
      const onMove = (e: MouseEvent) => {
        const nx = e.clientX / window.innerWidth - 0.5
        const ny = e.clientY / window.innerHeight - 0.5
        xTo(-nx * 16) // против курсора, max 8px
        yTo(-ny * 16)
      }
      window.addEventListener('mousemove', onMove)

      return () => {
        window.removeEventListener('mousemove', onMove)
        split.revert()
      }
    },
    { scope: rootRef },
  )

  return (
    <div
      ref={rootRef}
      className="relative h-full overflow-hidden bg-[#2E3160] bg-grad-mesh"
    >
      <MeshCanvas />
      <div
        ref={contentRef}
        className="relative z-10 flex h-full flex-col justify-center p-8 lg:p-16"
      >
        <img src="/logo.svg" alt="Everyday" className="auth-logo h-10 w-auto self-start" />
        <h1
          className="auth-display mt-10 text-white font-bold"
          style={{ fontSize: 'clamp(32px, 3.6vw, 56px)', lineHeight: 1.14, letterSpacing: '-0.02em' }}
        >
          Инструменты под контролем. Даже без интернета
        </h1>
        <p className="auth-subtitle mt-6 max-w-md text-[17px] leading-[26px] text-brand-100">
          Учёт инструментов и ТМЦ для строительных и производственных команд. Онлайн сегодня —
          с безопасными локальными данными и контролируемой синхронизацией.
        </p>
        <ul className="mt-10 space-y-4">
          {FEATURES.map(({ icon: Icon, text }) => (
            <li key={text} className="auth-feature flex items-center gap-3">
              <Icon size={24} strokeWidth={1.75} className="shrink-0 text-teal" />
              <span className="text-[15px] leading-[22px] text-white">{text}</span>
            </li>
          ))}
        </ul>
        <p className="auth-footer mt-12 text-xs text-ink-300">
          © Everyday · PWA для ежедневного учёта
        </p>
      </div>
    </div>
  )
}
