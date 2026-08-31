import { copyFileSync, existsSync, mkdirSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const target = path.join(root, '.build', 'cargo')
const result = spawnSync(
  'cargo',
  ['build', '--release', '--manifest-path', path.join(root, 'backend', 'Cargo.toml'), '--target-dir', target],
  { cwd: root, stdio: 'inherit' },
)

if (result.error) throw result.error
if (result.status !== 0) process.exit(result.status ?? 1)

const filename = process.platform === 'win32' ? 'meshkeeper-node.exe' : 'meshkeeper-node'
const source = path.join(target, 'release', filename)
if (!existsSync(source)) throw new Error(`Cargo did not produce ${source}`)
const output = path.join(root, 'dist', 'server')
mkdirSync(output, { recursive: true })
copyFileSync(source, path.join(output, filename))
