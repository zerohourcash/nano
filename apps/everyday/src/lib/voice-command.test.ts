import { describe, expect, it } from 'vitest'
import { normalizeVoiceMatch, parseVoiceCommand } from './voice-command'

describe('parseVoiceCommand', () => {
  it('extracts a chat transaction without changing its text', () => {
    expect(parseVoiceCommand('Отправь сообщение в чат: Смена закончена')).toEqual({
      kind: 'chat', text: 'Смена закончена',
    })
  })

  it('extracts inventory lookup and return intents', () => {
    expect(parseVoiceCommand('Найди инструмент шуруповёрт Bosch')).toEqual({ kind: 'find', query: 'шуруповёрт Bosch' })
    expect(parseVoiceCommand('Верни ТМЦ ВН-0042')).toEqual({ kind: 'return', query: 'ВН-0042' })
    expect(parseVoiceCommand('Возьми перфоратор')).toEqual({ kind: 'checkout', query: 'перфоратор' })
  })

  it('normalizes Russian matching and fails closed on an unknown command', () => {
    expect(normalizeVoiceMatch('Шуруповёрт — ВН-42')).toBe('шуруповерт вн 42')
    expect(parseVoiceCommand('удали всё')).toEqual({ kind: 'unknown', original: 'удали всё' })
  })
})
