import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const dist = path.join(root, 'dist')

const forbidden = /(?:\.db(?:-(?:wal|shm|journal))?|\.sqlite\d*|\.apk|\.aab|\.jks|\.keystore|\.pem|\.key)$/i
const textExt = new Set(['.html', '.js', '.css', '.json', '.map', '.txt', '.md'])

// Структурные маркеры: приватные ключи и снятая схема идентификации.
// Конкретные утёкшие строки здесь не хранятся намеренно — иначе файл-сторож
// сам публиковал бы секрет, который призван ловить.
const structural = [/BEGIN (?:OPENSSH |RSA |EC |DSA )?PRIVATE KEY/, /x-user-id/, /mk_user/]

/**
 * Дополнительные шаблоны для конкретного развёртывания: адреса, старые пароли
 * и прочее, что нельзя писать в репозиторий. По одному регулярному выражению
 * в строке; строки, начинающиеся с #, игнорируются.
 *
 * Файл не коммитится (см. .gitignore).
 */
function localPatterns() {
  const file = path.join(root, 'scripts', 'leaked-patterns.local')
  if (!existsSync(file)) return []
  return readFileSync(file, 'utf8')
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith('#'))
    .map((line) => new RegExp(line))
}

const patterns = [...structural, ...localPatterns()]
const failures = []

function walk(dir) {
  for (const name of readdirSync(dir)) {
    const file = path.join(dir, name)
    const stat = statSync(file)
    if (stat.isDirectory()) {
      walk(file)
      continue
    }
    const rel = path.relative(dist, file)
    if (forbidden.test(name) || /debug.*\.apk$/i.test(name)) {
      failures.push(`${rel}: forbidden artifact`)
    }
    if (textExt.has(path.extname(name).toLowerCase())) {
      const text = readFileSync(file, 'utf8')
      if (patterns.some((re) => re.test(text))) {
        failures.push(`${rel}: secret or retired identity marker`)
      }
    }
  }
}

walk(dist)
if (failures.length) {
  console.error(failures.join('\n'))
  process.exit(1)
}
const extra = patterns.length - structural.length
console.log(
  `Artifact allowlist check passed: no databases, keys, APKs, or retired credentials` +
    (extra > 0 ? ` (+${extra} local pattern${extra > 1 ? 's' : ''}).` : '.'),
)
