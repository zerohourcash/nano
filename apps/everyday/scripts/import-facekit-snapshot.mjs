import { createHash } from 'node:crypto'
import { mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'

const source = path.resolve(process.argv[2] ?? '../facekit/1')
const output = path.resolve(process.argv[3] ?? 'data/import/facekit-snapshot.json')

if (!statSync(source, { throwIfNoEntry: false })?.isDirectory()) {
  console.error(`Каталог снимков не найден: ${source}`)
  process.exit(1)
}

function decodeHtml(value) {
  const named = {
    amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ', laquo: '«', raquo: '»',
  }
  return value.replace(/&(#x?[0-9a-f]+|[a-z]+);/gi, (whole, key) => {
    if (key[0] === '#') {
      const hex = key[1]?.toLowerCase() === 'x'
      const value = Number.parseInt(key.slice(hex ? 2 : 1), hex ? 16 : 10)
      return Number.isFinite(value) ? String.fromCodePoint(value) : whole
    }
    return named[key.toLowerCase()] ?? whole
  })
}

function visibleText(html) {
  const body = html
    .replace(/<(script|style|noscript|svg)\b[^>]*>[\s\S]*?<\/\1>/gi, '\n')
    .replace(/<!--([\s\S]*?)-->/g, '\n')
    .replace(/<br\s*\/?>/gi, '\n')
    .replace(/<\/[^>]+>/g, '\n')
    .replace(/<[^>]+>/g, ' ')
  return decodeHtml(body)
    .split(/\r?\n/)
    .map((line) => line.replace(/\s+/g, ' ').trim())
    .filter(Boolean)
}

const files = readdirSync(source)
  .filter((name) => name.toLowerCase().endsWith('.html'))
  .sort((a, b) => a.localeCompare(b, 'ru'))

const pages = files.map((name) => {
  const file = path.join(source, name)
  const raw = readFileSync(file, 'utf8')
  const text = visibleText(raw)
  return {
    page: path.basename(name, path.extname(name)),
    sourceFile: name,
    sourceSha256: createHash('sha256').update(raw).digest('hex'),
    visibleText: text,
    uniqueText: [...new Set(text)],
  }
})

const payload = {
  schema: 'secure-equipment/facekit-snapshot-v1',
  importedAt: new Date().toISOString(),
  source: 'FaceKit saved HTML pages',
  warning: 'Снимок содержит только данные, отрисованные в браузере в момент сохранения, а не полную базу FaceKit.',
  pages,
}

mkdirSync(path.dirname(output), { recursive: true })
writeFileSync(output, `${JSON.stringify(payload, null, 2)}\n`, { mode: 0o600 })
console.log(`Извлечено ${pages.length} страниц → ${output}`)
