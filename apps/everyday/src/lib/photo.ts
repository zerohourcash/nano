/**
 * Подготовка снимка к отправке (ТЗ §5: вложение хранится вместе с уменьшенной
 * копией).
 *
 * Фотографии лежат в базе как data-URL, поэтому размер имеет прямое значение:
 * оригинал с телефона — это несколько мегабайт, и без сжатия каждая карточка
 * раздувает и базу, и каждый ответ API. Уменьшаем оригинал до разумного
 * предела и отдельно делаем миниатюру для списков.
 */

/** Длинная сторона полноразмерного снимка. */
const FULL_MAX_SIDE = 1600
/** Длинная сторона миниатюры для каталога. */
const THUMB_MAX_SIDE = 320
const JPEG_QUALITY = 0.85

export interface PreparedPhoto {
  /** Полноразмерный снимок для карточки. */
  url: string
  /** Уменьшенная копия для списков. */
  thumbUrl: string
}

function drawScaled(source: CanvasImageSource, width: number, height: number, maxSide: number) {
  const scale = Math.min(1, maxSide / Math.max(width, height))
  const canvas = document.createElement('canvas')
  canvas.width = Math.max(1, Math.round(width * scale))
  canvas.height = Math.max(1, Math.round(height * scale))
  const ctx = canvas.getContext('2d')
  if (!ctx) return null
  ctx.drawImage(source, 0, 0, canvas.width, canvas.height)
  return canvas.toDataURL('image/jpeg', JPEG_QUALITY)
}

async function loadImage(file: File): Promise<{ source: CanvasImageSource; width: number; height: number }> {
  // createImageBitmap быстрее и не держит DOM-узел, но есть не везде.
  if (typeof createImageBitmap === 'function') {
    const bitmap = await createImageBitmap(file)
    return { source: bitmap, width: bitmap.width, height: bitmap.height }
  }
  const url = URL.createObjectURL(file)
  try {
    const image = await new Promise<HTMLImageElement>((resolve, reject) => {
      const el = new Image()
      el.onload = () => resolve(el)
      el.onerror = () => reject(new Error('Не удалось прочитать изображение'))
      el.src = url
    })
    return { source: image, width: image.naturalWidth, height: image.naturalHeight }
  } finally {
    URL.revokeObjectURL(url)
  }
}

function readAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result))
    reader.onerror = () => reject(new Error('Не удалось прочитать файл'))
    reader.readAsDataURL(file)
  })
}

/**
 * Возвращает пару «оригинал + миниатюра». Если браузер не смог обработать
 * изображение, отдаёт исходный файл как есть — снимок не должен потеряться
 * из-за неудачного сжатия.
 */
export async function preparePhoto(file: File): Promise<PreparedPhoto> {
  try {
    const { source, width, height } = await loadImage(file)
    const url = drawScaled(source, width, height, FULL_MAX_SIDE)
    const thumbUrl = drawScaled(source, width, height, THUMB_MAX_SIDE)
    if (url && thumbUrl) return { url, thumbUrl }
  } catch {
    /* падаем в запасной путь ниже */
  }
  const raw = await readAsDataUrl(file)
  return { url: raw, thumbUrl: raw }
}
