import { spawn } from 'node:child_process'
import { existsSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const binary = path.join(root, 'dist', 'server', process.platform === 'win32' ? 'meshkeeper-node.exe' : 'meshkeeper-node')

if (!existsSync(binary)) {
  console.error('Production server is not built. Run: npm run build')
  process.exit(1)
}

const child = spawn(binary, [], { cwd: root, stdio: 'inherit', env: process.env })
child.on('exit', (code, signal) => {
  if (signal) process.kill(process.pid, signal)
  else process.exit(code ?? 1)
})
