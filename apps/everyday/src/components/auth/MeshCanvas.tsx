import { memo, useEffect, useRef } from 'react'

/**
 * Анимированный mesh-фон экрана входа (auth.md «Mesh-фон (canvas)»):
 * ~40 узлов (2–3px, #66C6BE / #C9C9F0), медленный дрейф 0.15px/кадр,
 * линии между узлами ближе 140px (opacity по расстоянию, max 0.35),
 * 2 «пульса» — светящиеся точки пробегают по линиям (намёк на передачу блоков).
 * Курсор притягивает ближайшие узлы в радиусе 160px.
 * Под canvas всегда лежит статичный fallback /auth-bg-nodes.svg.
 */
interface Node {
  x: number
  y: number
  vx: number
  vy: number
  r: number
  color: string
  ox: number // смещение от притяжения курсора
  oy: number
}

interface Pulse {
  from: number
  to: number
  t: number // 0..1 вдоль линии
  speed: number
}

const NODE_COUNT = 40
const LINK_DIST = 140
const LINK_MAX_OPACITY = 0.35
const DRIFT = 0.15 // px/кадр
const CURSOR_RADIUS = 160
const COLORS = ['#66C6BE', '#C9C9F0']

const MeshCanvas = memo(function MeshCanvas() {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const wrapRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    const canvas = canvasRef.current
    const wrap = wrapRef.current
    if (!canvas || !wrap) return
    const ctx = canvas.getContext('2d')
    if (!ctx) return // остаётся SVG-fallback

    const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches

    let width = 0
    let height = 0
    let raf = 0
    let visible = true
    const dpr = Math.min(window.devicePixelRatio || 1, 2)

    const nodes: Node[] = []
    const pulses: Pulse[] = []
    const mouse = { x: -9999, y: -9999 }

    const seed = () => {
      nodes.length = 0
      for (let i = 0; i < NODE_COUNT; i++) {
        const angle = Math.random() * Math.PI * 2
        nodes.push({
          x: Math.random() * width,
          y: Math.random() * height,
          vx: Math.cos(angle) * DRIFT,
          vy: Math.sin(angle) * DRIFT,
          r: 2 + Math.random(),
          color: COLORS[i % COLORS.length],
          ox: 0,
          oy: 0,
        })
      }
      pulses.length = 0
      for (let i = 0; i < 2; i++) {
        pulses.push({ from: -1, to: -1, t: 0, speed: 0.004 + Math.random() * 0.003 })
      }
    }

    const resize = () => {
      const rect = wrap.getBoundingClientRect()
      width = rect.width
      height = rect.height
      canvas.width = Math.round(width * dpr)
      canvas.height = Math.round(height * dpr)
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
      if (nodes.length === 0) seed()
    }

    const findLink = (p: Pulse) => {
      // выбираем случайную пару узлов на связи
      for (let attempt = 0; attempt < 12; attempt++) {
        const a = Math.floor(Math.random() * nodes.length)
        const b = Math.floor(Math.random() * nodes.length)
        if (a === b) continue
        const dx = nodes[a].x - nodes[b].x
        const dy = nodes[a].y - nodes[b].y
        if (dx * dx + dy * dy < LINK_DIST * LINK_DIST) {
          p.from = a
          p.to = b
          p.t = 0
          return
        }
      }
      p.from = -1
      p.to = -1
    }

    const draw = () => {
      ctx.clearRect(0, 0, width, height)

      // линии
      for (let i = 0; i < nodes.length; i++) {
        for (let j = i + 1; j < nodes.length; j++) {
          const a = nodes[i]
          const b = nodes[j]
          const dx = a.x - b.x
          const dy = a.y - b.y
          const dist = Math.hypot(dx, dy)
          if (dist < LINK_DIST) {
            const opacity = LINK_MAX_OPACITY * (1 - dist / LINK_DIST)
            ctx.strokeStyle = `rgba(102, 198, 190, ${opacity.toFixed(3)})`
            ctx.lineWidth = 1
            ctx.beginPath()
            ctx.moveTo(a.x + a.ox, a.y + a.oy)
            ctx.lineTo(b.x + b.ox, b.y + b.oy)
            ctx.stroke()
          }
        }
      }

      // узлы
      for (const n of nodes) {
        ctx.fillStyle = n.color
        ctx.beginPath()
        ctx.arc(n.x + n.ox, n.y + n.oy, n.r, 0, Math.PI * 2)
        ctx.fill()
      }

      // пульсы — светящаяся точка пробегает по линии от узла к узлу
      for (const p of pulses) {
        if (p.from < 0 || p.to < 0) {
          findLink(p)
          continue
        }
        const a = nodes[p.from]
        const b = nodes[p.to]
        const dx = a.x - b.x
        const dy = a.y - b.y
        if (dx * dx + dy * dy >= LINK_DIST * LINK_DIST) {
          findLink(p)
          continue
        }
        const px = a.x + a.ox + (b.x + b.ox - a.x - a.ox) * p.t
        const py = a.y + a.oy + (b.y + b.oy - a.y - a.oy) * p.t
        const glow = ctx.createRadialGradient(px, py, 0, px, py, 8)
        glow.addColorStop(0, 'rgba(102, 198, 190, 0.9)')
        glow.addColorStop(1, 'rgba(102, 198, 190, 0)')
        ctx.fillStyle = glow
        ctx.beginPath()
        ctx.arc(px, py, 8, 0, Math.PI * 2)
        ctx.fill()
        ctx.fillStyle = '#AEF0EA'
        ctx.beginPath()
        ctx.arc(px, py, 2, 0, Math.PI * 2)
        ctx.fill()
      }
    }

    const tick = () => {
      for (const n of nodes) {
        n.x += n.vx
        n.y += n.vy
        if (n.x < -10) n.x = width + 10
        if (n.x > width + 10) n.x = -10
        if (n.y < -10) n.y = height + 10
        if (n.y > height + 10) n.y = -10

        // притяжение курсора (отдельное смещение с затуханием, базовая позиция не трогается)
        const dx = mouse.x - n.x
        const dy = mouse.y - n.y
        const dist = Math.hypot(dx, dy)
        if (dist < CURSOR_RADIUS && dist > 0.001) {
          const force = ((CURSOR_RADIUS - dist) / CURSOR_RADIUS) * 0.6
          n.ox += (dx / dist) * force
          n.oy += (dy / dist) * force
        }
        n.ox *= 0.92
        n.oy *= 0.92

        // изредка меняем направление дрейфа
        if (Math.random() < 0.002) {
          const angle = Math.random() * Math.PI * 2
          n.vx = Math.cos(angle) * DRIFT
          n.vy = Math.sin(angle) * DRIFT
        }
      }
      for (const p of pulses) {
        p.t += p.speed
        if (p.t >= 1) findLink(p)
      }
      draw()
      raf = requestAnimationFrame(tick)
    }

    const onMouseMove = (e: MouseEvent) => {
      const rect = canvas.getBoundingClientRect()
      mouse.x = e.clientX - rect.left
      mouse.y = e.clientY - rect.top
    }
    const onMouseLeave = () => {
      mouse.x = -9999
      mouse.y = -9999
    }

    const io = new IntersectionObserver(
      ([entry]) => {
        visible = entry.isIntersecting
        if (visible && !reduced) {
          cancelAnimationFrame(raf)
          raf = requestAnimationFrame(tick)
        } else if (!visible) {
          cancelAnimationFrame(raf)
        }
      },
      { threshold: 0.05 },
    )

    resize()
    const ro = new ResizeObserver(resize)
    ro.observe(wrap)
    io.observe(wrap)

    if (reduced) {
      // prefers-reduced-motion: один статичный кадр поверх fallback
      draw()
    } else {
      raf = requestAnimationFrame(tick)
      window.addEventListener('mousemove', onMouseMove)
      window.addEventListener('mouseleave', onMouseLeave)
    }

    return () => {
      cancelAnimationFrame(raf)
      ro.disconnect()
      io.disconnect()
      window.removeEventListener('mousemove', onMouseMove)
      window.removeEventListener('mouseleave', onMouseLeave)
    }
  }, [])

  return (
    <div
      ref={wrapRef}
      style={{ position: 'absolute', inset: 0, overflow: 'hidden', pointerEvents: 'none' }}
      aria-hidden="true"
    >
      {/* Статичный fallback (auth.md): виден до старта canvas и при отказе 2d-контекста */}
      <style>{`@keyframes mesh-canvas-fade { from { opacity: 0 } to { opacity: 1 } }`}</style>
      <img
        src="/auth-bg-nodes.svg"
        alt=""
        style={{
          position: 'absolute',
          inset: 0,
          width: '100%',
          height: '100%',
          objectFit: 'cover',
          opacity: 0.6,
        }}
      />
      <canvas
        ref={canvasRef}
        style={{
          position: 'absolute',
          top: 0,
          left: 0,
          width: '100%',
          height: '100%',
          animation: 'mesh-canvas-fade 800ms ease-out both',
        }}
      />
    </div>
  )
})

export default MeshCanvas
