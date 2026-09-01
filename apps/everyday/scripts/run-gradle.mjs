import { existsSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const android = path.join(root, 'android')
const windows = process.platform === 'win32'
const wrapper = path.join(android, windows ? 'gradlew.bat' : 'gradlew')
const args = process.argv.slice(2)

if (args.length === 0) {
  console.error('Usage: node scripts/run-gradle.mjs <task> [arguments]')
  process.exit(2)
}
if (!existsSync(wrapper)) {
  console.error(`Pinned Gradle wrapper is missing: ${wrapper}`)
  process.exit(2)
}

const env = { ...process.env }
// CI/dev images commonly install the SDK here. Explicit environment variables
// always win, so production builders can use any SDK location.
if (!env.ANDROID_HOME && !env.ANDROID_SDK_ROOT && existsSync('/opt/android-sdk')) {
  env.ANDROID_HOME = '/opt/android-sdk'
  env.ANDROID_SDK_ROOT = '/opt/android-sdk'
}

const result = spawnSync(wrapper, args, {
  cwd: android,
  env,
  stdio: 'inherit',
  shell: windows,
})
if (result.error) {
  console.error(result.error.message)
  process.exit(1)
}
process.exit(result.status ?? 1)
