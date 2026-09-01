import { createHash } from 'node:crypto'
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const platform = { win32: 'windows', darwin: 'macos', linux: 'linux' }[process.platform]
const arch = { x64: 'x86_64', arm64: 'arm64' }[process.arch] ?? process.arch
const output = path.join(root, '.release', `everyday-${platform}-${arch}`)
const manifestPath = path.join(output, 'release-manifest.json')
if (!existsSync(manifestPath)) throw new Error(`Missing ${manifestPath}`)
const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
if (manifest.format !== 'everyday-desktop-release/v1') throw new Error('Unexpected manifest format')
if (manifest.platform !== platform || manifest.arch !== arch) throw new Error('Platform mismatch')

function files(dir, prefix = '') {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const relative = path.posix.join(prefix, entry.name)
    return entry.isDirectory() ? files(path.join(dir, entry.name), relative) : [relative]
  })
}

const actual = files(output).filter((file) => file !== 'release-manifest.json').sort()
const declared = manifest.files.map((entry) => entry.path).sort()
if (JSON.stringify(actual) !== JSON.stringify(declared)) throw new Error('Manifest file set mismatch')
for (const entry of manifest.files) {
  const file = path.join(output, ...entry.path.split('/'))
  const bytes = readFileSync(file)
  const digest = createHash('sha256').update(bytes).digest('hex')
  if (bytes.length !== entry.size || digest !== entry.sha256) {
    throw new Error(`Checksum mismatch: ${entry.path}`)
  }
  if (/\.(?:db|sqlite\d*|apk|aab|jks|keystore|pem|key)$/i.test(entry.path)) {
    throw new Error(`Forbidden private/runtime artifact: ${entry.path}`)
  }
}
if (!existsSync(path.join(output, ...manifest.binary.split('/')))) throw new Error('Binary is absent')
if (!actual.includes('dist/public/index.html')) throw new Error('Offline UI is absent')
if (statSync(path.join(output, ...manifest.binary.split('/'))).size === 0) throw new Error('Binary is empty')
console.log(`Desktop package verified: ${platform}/${arch}, ${actual.length} files`)
