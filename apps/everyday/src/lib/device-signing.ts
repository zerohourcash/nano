const DB_NAME = 'everyday-device-keys-v1'
const STORE = 'keys'
const RECORD = 'primary'
const DOMAIN = 'everyday/device-request/v1'

type DeviceIdentity = {
  deviceId: string
  privateKey: CryptoKey
  publicKey: Uint8Array
}

function encode(bytes: ArrayBuffer | Uint8Array): string {
  const array = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes)
  let binary = ''
  for (const byte of array) binary += String.fromCharCode(byte)
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
}

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, 1)
    request.onupgradeneeded = () => request.result.createObjectStore(STORE)
    request.onsuccess = () => resolve(request.result)
    request.onerror = () => reject(request.error)
  })
}

async function storedIdentity(): Promise<DeviceIdentity | undefined> {
  const db = await openDb()
  return new Promise((resolve, reject) => {
    const request = db.transaction(STORE).objectStore(STORE).get(RECORD)
    request.onsuccess = () => resolve(request.result as DeviceIdentity | undefined)
    request.onerror = () => reject(request.error)
  })
}

async function saveIdentity(identity: DeviceIdentity): Promise<void> {
  const db = await openDb()
  await new Promise<void>((resolve, reject) => {
    const request = db.transaction(STORE, 'readwrite').objectStore(STORE).put(identity, RECORD)
    request.onsuccess = () => resolve()
    request.onerror = () => reject(request.error)
  })
}

async function identity(): Promise<DeviceIdentity> {
  const stored = await storedIdentity()
  if (stored) return stored
  const generated = (await crypto.subtle.generateKey('Ed25519', true, ['sign', 'verify'])) as CryptoKeyPair
  const privatePkcs8 = await crypto.subtle.exportKey('pkcs8', generated.privateKey)
  const publicRaw = new Uint8Array(await crypto.subtle.exportKey('raw', generated.publicKey))
  const privateKey = await crypto.subtle.importKey('pkcs8', privatePkcs8, 'Ed25519', false, ['sign'])
  const created = { deviceId: crypto.randomUUID(), privateKey, publicKey: publicRaw }
  await saveIdentity(created)
  return created
}

async function registerDevice(current: DeviceIdentity): Promise<void> {
  const response = await fetch('/api/trpc/auth.registerDevice', {
    method: 'POST',
    credentials: 'include',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      json: {
        deviceId: current.deviceId,
        name: navigator.userAgent.includes('Android') ? 'Телефон Android' : 'Это устройство',
        publicKey: encode(current.publicKey),
      },
    }),
  })
  if (!response.ok) throw new Error('Не удалось зарегистрировать криптографический ключ устройства')
}

const CRITICAL = [
  'items.create',
  'items.update',
  'transfers.take',
  'transfers.takeMany',
  'transfers.returnItem',
  'transfers.prepare',
  'transfers.accept',
  'transfers.reject',
  'history.writeOff',
  'history.replenish',
  'history.move',
  'inventory.checkItem',
  'inventory.complete',
  'chat.send',
  'items.addDocument',
  'bit.transfer',
  'bit.sale',
  'bit.mint',
  'knowledge.save',
  'sync.importBundle',
  'sync.clearDiagnostics',
  'sync.reportTransportStatus',
  'content.setMode',
  'admin.users.create',
  'admin.users.update',
  'admin.users.remove',
  'admin.users.invite',
  'admin.workspaces.create',
  'admin.workspaces.update',
  'admin.workspaces.remove',
  'admin.workspaces.createInvite',
  'admin.organizationNodes.create',
  'admin.organizationNodes.update',
  'admin.organizationNodes.remove',
]

export function requiresDeviceSignature(url: string): boolean {
  return CRITICAL.some((procedure) => url.includes(procedure))
}

export async function signedDeviceHeaders(url: string, body: BodyInit | null | undefined): Promise<Record<string, string>> {
  if (typeof body !== 'string') throw new Error('Подписываемая операция должна иметь JSON-тело')
  const current = await identity()
  await registerDevice(current)
  const timestamp = Math.floor(Date.now() / 1000).toString()
  const nonce = crypto.randomUUID()
  const path = new URL(url, window.location.origin).pathname
  const digestBytes = new Uint8Array(
    await crypto.subtle.digest('SHA-256', new TextEncoder().encode(body))
  )
  const bodyHashHex = Array.from(digestBytes, (byte) => byte.toString(16).padStart(2, '0')).join('')
  const message = [DOMAIN, 'POST', path, timestamp, nonce, bodyHashHex].join('\n')
  const signature = await crypto.subtle.sign('Ed25519', current.privateKey, new TextEncoder().encode(message))
  return {
    'x-everyday-device': current.deviceId,
    'x-everyday-timestamp': timestamp,
    'x-everyday-nonce': nonce,
    'x-everyday-signature': encode(signature),
  }
}
