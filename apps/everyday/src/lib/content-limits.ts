// Base64 увеличивает тело примерно на треть. При HTTP-лимите узла 32 МиБ
// безопасный браузерный предел оставляет место для JSON и device-proof.
export const BROWSER_FILE_LIMIT_BYTES = 20 * 1024 * 1024
export const BROWSER_FILE_LIMIT_LABEL = '20 МБ'
