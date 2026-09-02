export type VoiceIntent =
  | { kind: 'chat'; text: string }
  | { kind: 'find'; query: string }
  | { kind: 'return'; query: string }
  | { kind: 'checkout'; query: string }
  | { kind: 'navigate'; path: string; label: string }
  | { kind: 'unknown'; original: string }

const clean = (value: string) => value.trim().replace(/[.!?]+$/u, '').replace(/\s+/gu, ' ')

export function parseVoiceCommand(raw: string): VoiceIntent {
  const original = clean(raw)
  const value = original.toLocaleLowerCase('ru-RU')
  const chat = original.match(/^(?:отправь|отправить|напиши|написать)(?:\s+сообщение)?\s+(?:в\s+)?(?:общий\s+)?чат(?:\s*[:,-])?\s+(.+)$/iu)
  if (chat?.[1]) return { kind: 'chat', text: clean(chat[1]) }

  const action = original.match(/^(найди|покажи|открой|верни|вернуть|возврати|возвратить|возьми|взять)(?:\s+(?:тмц|инструмент|оборудование|вещь))?\s+(.+)$/iu)
  if (action?.[2]) {
    const verb = action[1].toLocaleLowerCase('ru-RU')
    const query = clean(action[2])
    if (verb.startsWith('вер')) return { kind: 'return', query }
    if (verb.startsWith('возвр')) return { kind: 'return', query }
    if (verb.startsWith('возь') || verb === 'взять') return { kind: 'checkout', query }
    return { kind: 'find', query }
  }

  const routes: Array<[RegExp, string, string]> = [
    [/^(?:открой|покажи)\s+(?:общий\s+)?чат$/u, '/chat', 'чат организации'],
    [/^(?:открой|покажи)\s+(?:мой\s+)?кошел[её]к$/u, '/bit', 'кошелёк Bit'],
    [/^(?:открой|покажи)\s+(?:мой|мои)\s+(?:тмц|инструменты)$/u, '/my', 'мои ТМЦ'],
    [/^(?:открой|покажи)\s+инвентаризацию$/u, '/inventory', 'инвентаризацию'],
  ]
  for (const [pattern, path, label] of routes) {
    if (pattern.test(value)) return { kind: 'navigate', path, label }
  }
  return { kind: 'unknown', original }
}

export function normalizeVoiceMatch(value: string): string {
  return value.toLocaleLowerCase('ru-RU').replace(/ё/gu, 'е').replace(/[^\p{L}\p{N}]+/gu, ' ').trim()
}
