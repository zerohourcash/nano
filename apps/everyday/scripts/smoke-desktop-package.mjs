import { spawn } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const platform = { win32: 'windows', darwin: 'macos', linux: 'linux' }[process.platform]
const arch = { x64: 'x86_64', arm64: 'arm64' }[process.arch] ?? process.arch
const release = path.join(root, '.release', `everyday-${platform}-${arch}`)
const binary = path.join(
  release,
  'dist',
  'server',
  process.platform === 'win32' ? 'meshkeeper-node.exe' : 'meshkeeper-node',
)
const temporary = mkdtempSync(path.join(os.tmpdir(), 'everyday-desktop-smoke-'))

const port = await new Promise((resolve, reject) => {
  const server = net.createServer()
  server.once('error', reject)
  server.listen(0, '127.0.0.1', () => {
    const address = server.address()
    server.close(() => resolve(address.port))
  })
})
const child = spawn(binary, [], {
  cwd: release,
  env: {
    ...process.env,
    MESHKEEPER_BIND: `127.0.0.1:${port}`,
    MESHKEEPER_DB: path.join(temporary, 'node.db'),
    MESHKEEPER_NO_SEED: '1',
  },
  stdio: ['ignore', 'pipe', 'pipe'],
})
let stderr = ''
child.stderr.on('data', (chunk) => { stderr += chunk.toString() })
try {
  let healthy = false
  for (let attempt = 0; attempt < 300; attempt += 1) {
    if (child.exitCode !== null) throw new Error(`Packaged node exited early: ${stderr}`)
    try {
      const response = await fetch(`http://127.0.0.1:${port}/health`)
      healthy = response.ok
      if (healthy) break
    } catch { /* listener is not ready yet */ }
    await new Promise((resolve) => setTimeout(resolve, 50))
  }
  if (!healthy) throw new Error(`Packaged node did not become healthy: ${stderr}`)
  const page = await fetch(`http://127.0.0.1:${port}/`)
  const html = await page.text()
  if (!page.ok || !html.includes('<div id="root">')) throw new Error('Packaged offline UI is unavailable')
  console.log(`Desktop package smoke passed: ${platform}/${arch}`)
} finally {
  child.kill()
  await new Promise((resolve) => child.once('exit', resolve))
  rmSync(temporary, { recursive: true, force: true })
}
