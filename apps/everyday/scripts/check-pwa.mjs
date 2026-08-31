import { existsSync, readFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const manifest = JSON.parse(readFileSync(path.join(root, 'public', 'manifest.webmanifest'), 'utf8'))

function assert(condition, message) {
  if (!condition) throw new Error(`PWA check failed: ${message}`)
}

function pngSize(file) {
  const data = readFileSync(file)
  assert(data.toString('ascii', 1, 4) === 'PNG', `${file} is not PNG`)
  return [data.readUInt32BE(16), data.readUInt32BE(20)]
}

assert(manifest.id === '/', 'manifest id must be stable')
assert(manifest.name.startsWith('Everyday'), 'manifest must use Everyday brand')
assert(manifest.display === 'standalone', 'standalone display is required')
assert(Array.isArray(manifest.shortcuts) && manifest.shortcuts.length >= 4, 'operational shortcuts are required')

for (const icon of manifest.icons ?? []) {
  const file = path.join(root, 'public', icon.src.replace(/^\//, ''))
  assert(existsSync(file), `missing icon ${icon.src}`)
  const expected = icon.sizes.split('x').map(Number)
  const actual = pngSize(file)
  assert(expected[0] === actual[0] && expected[1] === actual[1], `${icon.src} has wrong dimensions`)
}

const sw = readFileSync(path.join(root, 'public', 'sw.js'), 'utf8')
assert(sw.includes("importScripts('/precache-manifest.js')"), 'service worker must load versioned precache')
assert(sw.includes("url.pathname.startsWith('/api')"), 'API must be excluded from static cache')
assert(sw.includes('PRECACHED_PATHS.has(url.pathname)'), 'runtime cache must be limited to public app-shell files')

const offline = readFileSync(path.join(root, 'public', 'offline.html'), 'utf8')
assert(!/\son\w+=/i.test(offline), 'offline page must not use inline event handlers blocked by CSP')

const builtPrecache = path.join(root, 'dist', 'public', 'precache-manifest.js')
assert(existsSync(builtPrecache), 'production precache manifest was not generated')
const precache = readFileSync(builtPrecache, 'utf8')
assert(precache.includes('/index.html') && precache.includes('/offline.html'), 'offline shell is incomplete')

console.log('Everyday PWA check passed: manifest, icons, offline shell and update cache are valid.')
