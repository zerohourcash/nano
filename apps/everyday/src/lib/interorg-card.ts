export type ContactCard = {
  v: 1
  type: 'everyday-org'
  workspaceGuid: string
  name: string
  encryptionKey: string
  signingKey: string
}

export function encodeCard(card: ContactCard): string {
  const bytes = new TextEncoder().encode(JSON.stringify(card))
  let binary = ''
  bytes.forEach((byte) => { binary += String.fromCharCode(byte) })
  return `everyday:org:${btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')}`
}

export function decodeCard(value: string): ContactCard {
  const raw = value.trim().replace(/^everyday:org:/, '').replace(/-/g, '+').replace(/_/g, '/')
  const padded = raw + '='.repeat((4 - raw.length % 4) % 4)
  const binary = atob(padded)
  const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0))
  const card = JSON.parse(new TextDecoder().decode(bytes)) as Partial<ContactCard>
  if (card.v !== 1 || card.type !== 'everyday-org' || !card.workspaceGuid || !card.encryptionKey || !card.signingKey) {
    throw new Error('Это не визитка организации Everyday')
  }
  return card as ContactCard
}
