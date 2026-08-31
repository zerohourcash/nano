import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')

function run(cmd, args, cwd) {
  const child = spawn(cmd, args, { cwd, stdio: 'inherit', shell: true })
  child.on('exit', (code) => {
    if (code && code !== 0) process.exitCode = code
  })
  return child
}

const cargo = run('cargo', ['run', '--release', '--manifest-path', 'backend/Cargo.toml'], root)
const vite = run('node', ['node_modules/vite/bin/vite.js', '--host', '--port', '3000'], root)

function stop() {
  cargo.kill()
  vite.kill()
}
process.on('SIGINT', stop)
process.on('SIGTERM', stop)
