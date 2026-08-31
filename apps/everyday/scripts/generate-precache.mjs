import { createHash } from 'node:crypto'
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const output = path.join(root, 'dist', 'public')
const excluded = new Set(['precache-manifest.js', 'sw.js'])
const largeRaster = /\.(?:avif|jpe?g|png|webp)$/i

function walk(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const absolute = path.join(directory, entry.name)
    if (entry.isDirectory()) return walk(absolute)
    const relative = path.relative(output, absolute).split(path.sep).join('/')
    if (excluded.has(relative)) return []
    // Тяжёлые демонстрационные фото загружаются по требованию. Это сохраняет
    // быструю установку PWA и не превращает app-shell в хранилище фотографий.
    if (largeRaster.test(relative) && statSync(absolute).size > 512_000) return []
    return [relative]
  })
}

const files = walk(output).sort()
const hash = createHash('sha256')
for (const file of files) {
  hash.update(file)
  hash.update(readFileSync(path.join(output, file)))
}

const manifest = {
  version: hash.digest('hex').slice(0, 16),
  files: files.map((file) => `/${file}`),
}

writeFileSync(
  path.join(output, 'precache-manifest.js'),
  `self.__EVERYDAY_PRECACHE__ = ${JSON.stringify(manifest)};\n`,
)
console.log(`Everyday PWA precache: ${files.length} files, version ${manifest.version}`)
