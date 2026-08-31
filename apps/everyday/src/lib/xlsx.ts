/**
 * Минимальный писатель XLSX (ТЗ §11, п. 8 — экспорт отчётов в Excel).
 *
 * Раньше «XLSX» был HTML-таблицей с расширением .xls: Excel её открывает, но
 * это не электронная таблица — числа приезжают текстом, а другие программы
 * файл не принимают. Здесь собирается настоящий xlsx.
 *
 * Зависимостей нет намеренно: формат — это ZIP с несколькими XML внутри, а
 * записи кладём без сжатия (метод store), что Excel поддерживает. Это дешевле
 * и безопаснее, чем тянуть в бандл целую библиотеку ради одной кнопки.
 */

// ─── CRC32 (нужен для заголовков ZIP) ────────────────────────────────────────

const CRC_TABLE = (() => {
  const table = new Uint32Array(256)
  for (let i = 0; i < 256; i++) {
    let c = i
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    table[i] = c >>> 0
  }
  return table
})()

function crc32(bytes: Uint8Array): number {
  let c = 0xffffffff
  for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]!) & 0xff]! ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}

// ─── Сборка ZIP без сжатия ───────────────────────────────────────────────────

interface ZipEntry {
  name: string
  data: Uint8Array
}

function writeUint32(view: DataView, offset: number, value: number) {
  view.setUint32(offset, value, true)
}

function buildZip(entries: ZipEntry[]): Blob {
  const encoder = new TextEncoder()
  const locals: Uint8Array[] = []
  const centrals: Uint8Array[] = []
  let offset = 0

  for (const entry of entries) {
    const nameBytes = encoder.encode(entry.name)
    const sum = crc32(entry.data)

    const local = new Uint8Array(30 + nameBytes.length)
    const lv = new DataView(local.buffer)
    writeUint32(lv, 0, 0x04034b50) // сигнатура локального заголовка
    lv.setUint16(4, 20, true) // требуемая версия
    lv.setUint16(6, 0, true) // флаги
    lv.setUint16(8, 0, true) // метод 0 — без сжатия
    lv.setUint16(10, 0, true) // время
    lv.setUint16(12, 0, true) // дата
    writeUint32(lv, 14, sum)
    writeUint32(lv, 18, entry.data.length)
    writeUint32(lv, 22, entry.data.length)
    lv.setUint16(26, nameBytes.length, true)
    lv.setUint16(28, 0, true) // extra
    local.set(nameBytes, 30)

    const central = new Uint8Array(46 + nameBytes.length)
    const cv = new DataView(central.buffer)
    writeUint32(cv, 0, 0x02014b50) // сигнатура записи каталога
    cv.setUint16(4, 20, true)
    cv.setUint16(6, 20, true)
    cv.setUint16(8, 0, true)
    cv.setUint16(10, 0, true)
    cv.setUint16(12, 0, true)
    cv.setUint16(14, 0, true)
    writeUint32(cv, 16, sum)
    writeUint32(cv, 20, entry.data.length)
    writeUint32(cv, 24, entry.data.length)
    cv.setUint16(28, nameBytes.length, true)
    writeUint32(cv, 42, offset)
    central.set(nameBytes, 46)

    locals.push(local, entry.data)
    centrals.push(central)
    offset += local.length + entry.data.length
  }

  const centralSize = centrals.reduce((sum, c) => sum + c.length, 0)
  const end = new Uint8Array(22)
  const ev = new DataView(end.buffer)
  writeUint32(ev, 0, 0x06054b50) // сигнатура конца каталога
  ev.setUint16(8, entries.length, true)
  ev.setUint16(10, entries.length, true)
  writeUint32(ev, 12, centralSize)
  writeUint32(ev, 16, offset)

  // Склеиваем в один буфер: так и Blob получает единственный кусок, и типы
  // типизированных массивов не расходятся между версиями TypeScript.
  const parts = [...locals, ...centrals, end]
  const total = parts.reduce((sum, part) => sum + part.length, 0)
  const out = new Uint8Array(total)
  let cursor = 0
  for (const part of parts) {
    out.set(part, cursor)
    cursor += part.length
  }
  return new Blob([out], {
    type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  })
}

// ─── XML листа ───────────────────────────────────────────────────────────────

function xmlEscape(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

/** A, B, … Z, AA, AB — имя столбца по индексу. */
export function columnName(index: number): string {
  let name = ''
  let n = index
  do {
    name = String.fromCharCode(65 + (n % 26)) + name
    n = Math.floor(n / 26) - 1
  } while (n >= 0)
  return name
}

export type CellValue = string | number | null | undefined

function cellXml(value: CellValue, ref: string): string {
  if (value === null || value === undefined || value === '') return ''
  if (typeof value === 'number' && Number.isFinite(value)) {
    return `<c r="${ref}"><v>${value}</v></c>`
  }
  // inlineStr избавляет от отдельной таблицы строк.
  return `<c r="${ref}" t="inlineStr"><is><t xml:space="preserve">${xmlEscape(String(value))}</t></is></c>`
}

function sheetXml(rows: CellValue[][]): string {
  const body = rows
    .map((row, rowIndex) => {
      const cells = row
        .map((value, colIndex) => cellXml(value, `${columnName(colIndex)}${rowIndex + 1}`))
        .join('')
      return `<row r="${rowIndex + 1}">${cells}</row>`
    })
    .join('')
  return (
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
    '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">' +
    `<sheetData>${body}</sheetData></worksheet>`
  )
}

/** Собирает книгу с одним листом. */
export function buildXlsx(rows: CellValue[][], sheetName = 'Отчёт'): Blob {
  const encoder = new TextEncoder()
  const file = (name: string, xml: string): ZipEntry => ({ name, data: encoder.encode(xml) })

  return buildZip([
    file(
      '[Content_Types].xml',
      '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">' +
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>' +
        '<Default Extension="xml" ContentType="application/xml"/>' +
        '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>' +
        '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>' +
        '</Types>',
    ),
    file(
      '_rels/.rels',
      '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>' +
        '</Relationships>',
    ),
    file(
      'xl/workbook.xml',
      '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
        '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" ' +
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">' +
        `<sheets><sheet name="${xmlEscape(sheetName.slice(0, 31))}" sheetId="1" r:id="rId1"/></sheets>` +
        '</workbook>',
    ),
    file(
      'xl/_rels/workbook.xml.rels',
      '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>' +
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' +
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>' +
        '</Relationships>',
    ),
    file('xl/worksheets/sheet1.xml', sheetXml(rows)),
  ])
}
