import { createHash } from 'node:crypto'
import {
  chmodSync,
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const platform = { win32: 'windows', darwin: 'macos', linux: 'linux' }[process.platform]
if (!platform) throw new Error(`Unsupported desktop platform: ${process.platform}`)
const arch = { x64: 'x86_64', arm64: 'arm64' }[process.arch] ?? process.arch
const name = `everyday-${platform}-${arch}`
const output = path.join(root, '.release', name)
const binaryName = process.platform === 'win32' ? 'meshkeeper-node.exe' : 'meshkeeper-node'
const binary = path.join(root, 'dist', 'server', binaryName)
const web = path.join(root, 'dist', 'public')
if (!existsSync(binary) || !existsSync(web)) {
  throw new Error('Production build is missing; run npm run build first')
}

rmSync(output, { recursive: true, force: true })
mkdirSync(path.join(output, 'dist', 'server'), { recursive: true })
cpSync(binary, path.join(output, 'dist', 'server', binaryName))
cpSync(web, path.join(output, 'dist', 'public'), { recursive: true })
writeFileSync(
  path.join(output, 'start.sh'),
  '#!/bin/sh\nset -eu\ncd "$(dirname "$0")"\nexec ./dist/server/meshkeeper-node "$@"\n',
)
chmodSync(path.join(output, 'start.sh'), 0o755)
writeFileSync(
  path.join(output, 'start.cmd'),
  '@echo off\r\ncd /d "%~dp0"\r\ndist\\server\\meshkeeper-node.exe %*\r\n',
)
writeFileSync(
  path.join(output, 'README.txt'),
  [
    'Everyday / Bit autonomous desktop node',
    '',
    platform === 'windows' ? 'Start: double-click start.cmd' : 'Start: ./start.sh',
    'Open: http://127.0.0.1:8080',
    'Data is stored locally in data/meshkeeper-rs.db.',
    'For LAN/HTTPS/mesh configuration see the project README and docs/SECURITY.md.',
    '',
  ].join('\n'),
)

function files(dir, prefix = '') {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const relative = path.posix.join(prefix, entry.name)
    return entry.isDirectory() ? files(path.join(dir, entry.name), relative) : [relative]
  })
}

const entries = files(output)
  .filter((file) => file !== 'release-manifest.json')
  .sort()
  .map((file) => {
    const bytes = readFileSync(path.join(output, ...file.split('/')))
    return {
      path: file,
      size: statSync(path.join(output, ...file.split('/'))).size,
      sha256: createHash('sha256').update(bytes).digest('hex'),
    }
  })
const manifest = {
  format: 'everyday-desktop-release/v1',
  platform,
  arch,
  binary: `dist/server/${binaryName}`,
  files: entries,
}
writeFileSync(path.join(output, 'release-manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`)
console.log(output)
