import { useEffect, useState } from 'react'
import { Download, RefreshCw, WifiOff, X } from 'lucide-react'

interface InstallPromptEvent extends Event {
  prompt: () => Promise<void>
  userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>
}

function isIosInstallCandidate() {
  const ios = /iphone|ipad|ipod/i.test(navigator.userAgent)
  const standalone = window.matchMedia('(display-mode: standalone)').matches
    || Boolean((navigator as Navigator & { standalone?: boolean }).standalone)
  return ios && !standalone && localStorage.getItem('everyday:ios-install-hidden') !== '1'
}

export default function PwaStatus() {
  const [online, setOnline] = useState(navigator.onLine)
  const [installPrompt, setInstallPrompt] = useState<InstallPromptEvent | null>(null)
  const [update, setUpdate] = useState<ServiceWorkerRegistration | null>(null)
  const [showIosHelp, setShowIosHelp] = useState(isIosInstallCandidate)

  useEffect(() => {
    const goOnline = () => setOnline(true)
    const goOffline = () => setOnline(false)
    const offerInstall = (event: Event) => {
      event.preventDefault()
      setInstallPrompt(event as InstallPromptEvent)
    }
    const updateReady = (event: Event) => {
      setUpdate((event as CustomEvent<ServiceWorkerRegistration>).detail)
    }
    const installed = () => setInstallPrompt(null)

    window.addEventListener('online', goOnline)
    window.addEventListener('offline', goOffline)
    window.addEventListener('beforeinstallprompt', offerInstall)
    window.addEventListener('appinstalled', installed)
    window.addEventListener('everyday:pwa-update', updateReady)
    return () => {
      window.removeEventListener('online', goOnline)
      window.removeEventListener('offline', goOffline)
      window.removeEventListener('beforeinstallprompt', offerInstall)
      window.removeEventListener('appinstalled', installed)
      window.removeEventListener('everyday:pwa-update', updateReady)
    }
  }, [])

  async function install() {
    if (!installPrompt) return
    await installPrompt.prompt()
    await installPrompt.userChoice
    setInstallPrompt(null)
  }

  function applyUpdate() {
    const worker = update?.waiting
    if (!worker) return
    let reloading = false
    navigator.serviceWorker.addEventListener('controllerchange', () => {
      if (!reloading) {
        reloading = true
        window.location.reload()
      }
    })
    worker.postMessage({ type: 'SKIP_WAITING' })
  }

  function hideIosHelp() {
    localStorage.setItem('everyday:ios-install-hidden', '1')
    setShowIosHelp(false)
  }

  if (online && !installPrompt && !update && !showIosHelp) return null

  return (
    <aside className="fixed inset-x-3 bottom-20 z-[70] mx-auto flex max-w-xl items-center gap-3 rounded-2xl border border-ink-200 bg-white p-3 shadow-xl md:bottom-4" aria-live="polite">
      {!online ? <WifiOff className="size-5 shrink-0 text-amber-600" aria-hidden="true" /> : update ? <RefreshCw className="size-5 shrink-0 text-brand" aria-hidden="true" /> : <Download className="size-5 shrink-0 text-brand" aria-hidden="true" />}
      <p className="min-w-0 flex-1 text-sm text-ink-700">
        {!online
          ? 'Нет связи с узлом. Интерфейс доступен, операции возобновятся после подключения.'
          : update
            ? 'Доступна новая версия Everyday.'
            : showIosHelp
              ? 'Для установки на iPhone нажмите «Поделиться» → «На экран Домой».'
              : 'Установите Everyday для быстрого запуска и доступа к локальному узлу.'}
      </p>
      {online && update && <button type="button" onClick={applyUpdate} className="rounded-xl bg-brand px-3 py-2 text-sm font-semibold text-white">Обновить</button>}
      {online && installPrompt && !update && <button type="button" onClick={install} className="rounded-xl bg-brand px-3 py-2 text-sm font-semibold text-white">Установить</button>}
      {showIosHelp && online && !update && (
        <button type="button" onClick={hideIosHelp} className="rounded-lg p-1 text-ink-500 hover:bg-ink-100" aria-label="Скрыть подсказку">
          <X className="size-5" aria-hidden="true" />
        </button>
      )}
    </aside>
  )
}
