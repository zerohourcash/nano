import { useEffect, useRef } from 'react'
import { trpc } from '@/providers/trpc'
import { useToast } from './ui'

type NativeBridge = {
  takePendingSyncBundle?: () => string
  acknowledgePendingSyncBundle?: (accepted: boolean) => void
}

function bridge(): NativeBridge | undefined {
  return (window as Window & { MeshKeeperNative?: NativeBridge }).MeshKeeperNative
}

/**
 * Durable Android transport handoff shared by all administration sections.
 * Java only stores opaque, bounded bytes. The signed Rust API decides whether
 * they are an organization sync bundle or an inter-organization gossip batch.
 */
export default function NativeTransportInbox() {
  const toast = useToast()
  const pending = useRef(false)
  const utils = trpc.useUtils()
  const importSync = trpc.sync.importBundle.useMutation()
  const importInterorg = trpc.interorg.importGossip.useMutation()

  useEffect(() => {
    const consume = async () => {
      if (pending.current) return
      const native = bridge()
      const raw = native?.takePendingSyncBundle?.()
      if (!raw) return
      pending.current = true
      try {
        const bundle = JSON.parse(raw) as { format?: unknown }
        if (bundle.format === 'everyday-sync-bundle') {
          const result = await importSync.mutateAsync({ bundle })
          toast(`BLE sync проверен: импортировано ${result.imported ?? 0}`)
        } else if (bundle.format === 'everyday-interorg-gossip') {
          const result = await importInterorg.mutateAsync({ bundle: bundle as {
            format: 'everyday-interorg-gossip'
            version: 1
            envelopes: unknown[]
          } })
          toast(`BLE interorg проверен: новых ${result.stored}, доставлено ${result.delivered}`)
        } else {
          throw new Error('Android передал неизвестный transport-пакет Everyday')
        }
        native?.acknowledgePendingSyncBundle?.(true)
        await utils.invalidate()
        pending.current = false
        setTimeout(() => void consume(), 0)
      } catch (error) {
        native?.acknowledgePendingSyncBundle?.(false)
        pending.current = false
        toast(error instanceof Error ? error.message : 'Transport-пакет отклонён', 'error')
      }
    }
    void consume()
    const listener = () => void consume()
    window.addEventListener('meshkeeper-native-bundle', listener)
    return () => window.removeEventListener('meshkeeper-native-bundle', listener)
  }, [importInterorg, importSync, toast, utils])

  return null
}
