import { describe, expect, it } from 'vitest'
import { decodeCard, encodeCard } from '@/lib/interorg-card'

describe('organization contact card', () => {
  it('round-trips unicode without exposing ambiguous fields', () => {
    const card = {
      v: 1 as const,
      type: 'everyday-org' as const,
      workspaceGuid: '8ba1af69-1826-4a11-b431-a3a9377eb78d',
      name: 'Цех № 7',
      encryptionKey: 'A'.repeat(44),
      signingKey: 'B'.repeat(44),
    }
    expect(decodeCard(encodeCard(card))).toEqual(card)
  })

  it('rejects arbitrary QR text', () => {
    expect(() => decodeCard('https://attacker.invalid/invite')).toThrow()
  })
})
