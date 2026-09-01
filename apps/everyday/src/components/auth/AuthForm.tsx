import { useCallback, useRef, useState } from 'react'
import type { FormEvent, KeyboardEvent } from 'react'
import { useNavigate, useSearchParams } from 'react-router'
import { AnimatePresence, motion } from 'framer-motion'
import {
  Phone,
  Lock,
  Eye,
  EyeOff,
  UserRound,
  Building2,
  Globe2,
  Info,
  Loader2,
  Check,
  ChevronDown,
} from 'lucide-react'
import { toast } from 'sonner'
import { cn } from '@/lib/utils'
import { trpc } from '@/providers/trpc'
import { isNativeApp } from '@/lib/app-mode'
import QrScanner from '@/components/QrScanner'

/* ── Телефонная маска +7 (___) ___-__-__ ── */
function phoneDigits(raw: string): string {
  let d = raw.replace(/\D/g, '')
  if (d.startsWith('8')) d = '7' + d.slice(1)
  if (!d.startsWith('7')) d = '7' + d
  return d.slice(0, 11)
}

function formatPhone(digits: string): string {
  const p1 = digits.slice(1, 4)
  const p2 = digits.slice(4, 7)
  const p3 = digits.slice(7, 9)
  const p4 = digits.slice(9, 11)
  let out = '+7'
  if (p1) out += ` (${p1}`
  if (p1.length === 3) out += ')'
  if (p2) out += ` ${p2}`
  if (p3) out += `-${p3}`
  if (p4) out += `-${p4}`
  return out
}

const EASE = [0.22, 1, 0.36, 1] as [number, number, number, number]

const fieldVariants = {
  hidden: { opacity: 0, y: 16 },
  show: (i: number) => ({
    opacity: 1,
    y: 0,
    transition: { delay: i * 0.05, duration: 0.26, ease: EASE },
  }),
}

const shakeVariants = {
  idle: { x: 0 },
  // тряска ±6px, 3 итерации, 300ms (auth.md «Валидация»)
  shake: { x: [0, -6, 6, -6, 6, -6, 0], transition: { duration: 0.3 } },
}

type SubmitState = 'idle' | 'loading' | 'success'

/* ── Базовое поле ввода дизайн-системы (design.md §6 «Инпуты») ── */
interface FieldProps {
  label: string
  required?: boolean
  error?: string
  shake: boolean
  index: number
  hint?: string
  children: React.ReactNode
}

function Field({ label, required, error, shake, index, hint, children }: FieldProps) {
  return (
    <motion.div variants={fieldVariants} initial="hidden" animate="show" custom={index}>
      <motion.div variants={shakeVariants} animate={shake ? 'shake' : 'idle'}>
        <label className="mb-1.5 block text-[13px] leading-[18px] font-semibold text-ink-500">
          {label}
          {required && <span className="text-accent"> *</span>}
        </label>
        {children}
      </motion.div>
      <AnimatePresence initial={false}>
        {error ? (
          <motion.p
            key="err"
            initial={{ opacity: 0, y: -8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -8 }}
            transition={{ duration: 0.2, ease: EASE }}
            className="mt-1 text-xs leading-4 text-danger"
          >
            {error}
          </motion.p>
        ) : hint ? (
          <p className="mt-1 text-xs leading-4 text-ink-300">{hint}</p>
        ) : null}
      </AnimatePresence>
    </motion.div>
  )
}

const inputClass = (hasError: boolean, withIcon = true) =>
  cn(
    'h-11 w-full rounded-xl border bg-surface py-3 pr-4 text-[15px] text-ink-900 transition-colors placeholder:text-ink-300',
    withIcon ? 'pl-11' : 'pl-4',
    hasError
      ? 'border-danger focus:border-danger focus:ring-[3px] focus:ring-[#D6454522]'
      : 'border-brand-100 focus:border-brand-600 focus:ring-[3px] focus:ring-[#5E629B22]',
  )

/* ── Кастомный чекбокс (design.md §6): 20px, радиус 6px, галка 150ms ── */
function Checkbox({
  checked,
  onChange,
  label,
}: {
  checked: boolean
  onChange: (v: boolean) => void
  label: React.ReactNode
}) {
  return (
    <label
      className="flex cursor-pointer select-none items-center gap-2"
      onClick={(e) => {
        e.preventDefault()
        onChange(!checked)
      }}
    >
      <span
        role="checkbox"
        aria-checked={checked}
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === ' ' || e.key === 'Enter') {
            e.preventDefault()
            onChange(!checked)
          }
        }}
        className={cn(
          'flex h-5 w-5 items-center justify-center rounded-md border-[1.5px] transition-colors',
          checked ? 'border-brand-600 bg-brand-600' : 'border-brand-100 bg-surface',
        )}
      >
        <motion.svg
          width="12"
          height="12"
          viewBox="0 0 12 12"
          fill="none"
          initial={false}
          animate={{ scale: checked ? 1 : 0, opacity: checked ? 1 : 0 }}
          transition={{ duration: 0.15 }}
        >
          <path
            d="M2 6.5L4.8 9L10 3"
            stroke="#fff"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </motion.svg>
      </span>
      <span className="text-[13px] leading-[18px] text-ink-900">{label}</span>
    </label>
  )
}

/* ── Кнопка Primary lg с состояниями spinner → «Готово» ── */
function SubmitButton({
  state,
  children,
  variant = 'primary',
}: {
  state: SubmitState
  children: React.ReactNode
  variant?: 'primary' | 'secondary'
}) {
  const isPrimary = variant === 'primary'
  return (
    <motion.button
      type="submit"
      disabled={state !== 'idle'}
      whileTap={state === 'idle' ? { scale: 0.97 } : undefined}
      className={cn(
        'flex h-12 w-full items-center justify-center gap-2 rounded-xl text-sm font-semibold transition-all',
        state === 'success'
          ? 'bg-success text-white'
          : isPrimary
            ? 'bg-accent text-white hover:-translate-y-px hover:bg-accent-hover'
            : 'border border-brand-100 bg-surface text-ink-900 hover:bg-brand-50',
        state === 'loading' && 'opacity-90',
      )}
    >
      {state === 'loading' ? (
        <Loader2 size={18} className="animate-spin" />
      ) : state === 'success' ? (
        <>
          <Check size={18} /> Готово
        </>
      ) : (
        children
      )}
    </motion.button>
  )
}

/* ── OTP: 6 ячеек 40×48, mono 20px, автофокус, авто-переход, вставка целиком ── */
function OtpInput({ onComplete }: { onComplete: (code: string) => void }) {
  const [digits, setDigits] = useState<string[]>(Array(6).fill(''))
  const refs = useRef<(HTMLInputElement | null)[]>([])

  const update = (next: string[], focusIdx: number) => {
    setDigits(next)
    refs.current[focusIdx]?.focus()
    if (next.every((d) => d !== '')) onComplete(next.join(''))
  }

  const handleChange = (i: number, raw: string) => {
    const d = raw.replace(/\D/g, '')
    if (!d) return
    if (d.length > 1) {
      // вставка кода целиком
      const next = Array(6).fill('')
      for (let k = 0; k < 6; k++) next[k] = d[k] ?? ''
      update(next, Math.min(d.length, 5))
      return
    }
    const next = [...digits]
    next[i] = d
    update(next, Math.min(i + 1, 5))
  }

  const handleKeyDown = (i: number, e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Backspace') {
      e.preventDefault()
      const next = [...digits]
      if (next[i]) {
        next[i] = ''
        update(next, i)
      } else if (i > 0) {
        next[i - 1] = ''
        update(next, i - 1)
      }
    }
    if (e.key === 'ArrowLeft' && i > 0) refs.current[i - 1]?.focus()
    if (e.key === 'ArrowRight' && i < 5) refs.current[i + 1]?.focus()
  }

  const handlePaste = (e: React.ClipboardEvent<HTMLInputElement>) => {
    e.preventDefault()
    const d = e.clipboardData.getData('text').replace(/\D/g, '').slice(0, 6)
    if (!d) return
    const next = Array(6).fill('')
    for (let k = 0; k < 6; k++) next[k] = d[k] ?? ''
    update(next, Math.min(d.length, 5))
  }

  return (
    <div className="flex justify-between gap-2">
      {digits.map((d, i) => (
        <input
          key={i}
          ref={(el) => {
            refs.current[i] = el
          }}
          value={d}
          autoFocus={i === 0}
          inputMode="numeric"
          maxLength={6}
          onChange={(e) => handleChange(i, e.target.value)}
          onKeyDown={(e) => handleKeyDown(i, e)}
          onPaste={handlePaste}
          aria-label={`Цифра ${i + 1} из 6`}
          className={cn(
            'h-12 w-10 rounded-xl border border-brand-100 bg-surface text-center font-mono text-xl',
            'text-ink-900 focus:border-brand-600 focus:ring-[3px] focus:ring-[#5E629B22]',
          )}
        />
      ))}
    </div>
  )
}

/* ── Индикатор надёжности пароля: полоса 4px, красный→жёлтый→зелёный ── */
function PasswordStrength({ password }: { password: string }) {
  if (!password) return null
  let score = 0
  if (password.length >= 8) score++
  if (/[а-яa-z]/.test(password) && /[А-ЯA-Z]/.test(password)) score++
  if (/\d/.test(password)) score++
  if (/[^а-яa-zА-ЯA-Z0-9]/.test(password)) score++
  const level = password.length < 10 ? 1 : Math.max(1, score)
  const colors = ['#D64545', '#D64545', '#A87C0F', '#A87C0F', '#2E9E5B']
  const labels = ['', 'Слабый', 'Средний', 'Хороший', 'Надёжный']
  return (
    <div className="mt-2">
      <div className="h-1 w-full overflow-hidden rounded-full bg-brand-50">
        <motion.div
          className="h-full rounded-full"
          initial={false}
          animate={{ width: `${(level / 4) * 100}%`, backgroundColor: colors[level] }}
          transition={{ duration: 0.25, ease: EASE }}
        />
      </div>
      <p className="mt-1 text-xs leading-4 text-ink-500">{labels[level]} пароль</p>
    </div>
  )
}

/* ── Иконка глаза с морфингом eye↔eye-off (150ms) ── */
function EyeToggle({ shown, onToggle }: { shown: boolean; onToggle: () => void }) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-label={shown ? 'Скрыть пароль' : 'Показать пароль'}
      className="absolute right-3 top-1/2 -translate-y-1/2 text-ink-300 transition-colors hover:text-brand-600"
    >
      <AnimatePresence mode="wait" initial={false}>
        <motion.span
          key={shown ? 'off' : 'on'}
          initial={{ opacity: 0, scale: 0.6, rotate: -30 }}
          animate={{ opacity: 1, scale: 1, rotate: 0 }}
          exit={{ opacity: 0, scale: 0.6, rotate: 30 }}
          transition={{ duration: 0.15 }}
          className="block"
        >
          {shown ? <EyeOff size={18} /> : <Eye size={18} />}
        </motion.span>
      </AnimatePresence>
    </button>
  )
}

/* ═══════════════ Таб «Вход» ═══════════════ */
function LoginTab() {
  const navigate = useNavigate()
  const utils = trpc.useUtils()
  const directory = trpc.auth.directory.useQuery()
  const login = trpc.auth.login.useMutation()
  const joinAfter = trpc.auth.join.useMutation()
  const [mode, setMode] = useState<'password' | 'sms'>('password')
  const [phone, setPhone] = useState('')
  const [password, setPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [remember, setRemember] = useState(true)
  const [state, setState] = useState<SubmitState>('idle')
  const [smsState, setSmsState] = useState<SubmitState>('idle')
  const [codeSent, setCodeSent] = useState(false)
  const [errors, setErrors] = useState<{ phone?: string; password?: string }>({})
  const [shakeTick, setShakeTick] = useState(0)

  const digits = phoneDigits(phone)
  const phoneComplete = digits.length === 11

  const finish = async (userId?: number, phoneValue?: string) => {
    try {
      setState('loading')
      await login.mutateAsync({ userId, phone: phoneValue, password: password || undefined })
      const joinToken = new URLSearchParams(window.location.search).get('join')
      if (joinToken) {
        try {
          await joinAfter.mutateAsync({ token: joinToken })
        } catch {
          /* вход важнее вступления */
        }
      }
      await utils.invalidate()
      setState('success')
      window.setTimeout(() => navigate('/'), 300)
    } catch (err) {
      setState('idle')
      toast.error(err instanceof Error ? err.message : 'Не удалось войти')
    }
  }

  const submitPassword = (e: FormEvent) => {
    e.preventDefault()
    if (state !== 'idle') return
    const next: typeof errors = {}
    if (!phoneComplete) next.phone = 'Введите телефон полностью'
    if (!password) next.password = 'Введите пароль'
    setErrors(next)
    if (Object.keys(next).length > 0) {
      setShakeTick((t) => t + 1)
      if (phoneComplete && !password) toast.error('Неверный телефон или пароль')
      return
    }
    void finish(undefined, phone)
  }

  const requestCode = () => {
    if (!phoneComplete) {
      setErrors({ phone: 'Введите телефон полностью' })
      setShakeTick((t) => t + 1)
      return
    }
    setErrors({})
    setSmsState('loading')
    window.setTimeout(() => {
      setSmsState('idle')
      setCodeSent(true)
    }, 700)
  }

  const submitCode = (code: string) => {
    if (code.length !== 6) return
    void finish(undefined, phone)
  }

  const onPhoneChange = (raw: string) => {
    setPhone(formatPhone(phoneDigits(raw)))
    if (errors.phone) setErrors((p) => ({ ...p, phone: undefined }))
  }

  return (
    <form
      onSubmit={submitPassword}
      noValidate
      className="space-y-4"
      key={`login-${mode}-${shakeTick}`}
    >
      <Field
        index={0}
        label="Телефон"
        required
        error={errors.phone}
        shake={!!errors.phone}
      >
        <div className="relative">
          <Phone
            size={18}
            className="pointer-events-none absolute left-4 top-1/2 -translate-y-1/2 text-ink-300"
          />
          <input
            type="tel"
            value={phone}
            onChange={(e) => onPhoneChange(e.target.value)}
            placeholder="+7 (921) 555-01-42"
            autoComplete="tel"
            className={inputClass(!!errors.phone)}
          />
        </div>
      </Field>

      {mode === 'password' ? (
        <>
          <Field index={1} label="Пароль" error={errors.password} shake={!!errors.password}>
            <div className="relative">
              <Lock
                size={18}
                className="pointer-events-none absolute left-4 top-1/2 -translate-y-1/2 text-ink-300"
              />
              <input
                type={showPassword ? 'text' : 'password'}
                value={password}
                onChange={(e) => {
                  setPassword(e.target.value)
                  if (errors.password) setErrors((p) => ({ ...p, password: undefined }))
                }}
                placeholder="Ваш пароль"
                autoComplete="current-password"
                className={cn(inputClass(!!errors.password), 'pr-11')}
              />
              <EyeToggle shown={showPassword} onToggle={() => setShowPassword((v) => !v)} />
            </div>
          </Field>

          <motion.div
            variants={fieldVariants}
            initial="hidden"
            animate="show"
            custom={2}
            className="flex items-center justify-between"
          >
            <Checkbox checked={remember} onChange={setRemember} label="Запомнить меня" />
            <button
              type="button"
              onClick={() => toast.message('Ссылка для сброса отправлена по SMS')}
              className="rounded-md px-1 text-[13px] font-semibold text-brand-600 transition-colors hover:bg-brand-50"
            >
              Забыли пароль?
            </button>
          </motion.div>

          <motion.div variants={fieldVariants} initial="hidden" animate="show" custom={3}>
            <SubmitButton state={state}>Войти</SubmitButton>
          </motion.div>
        </>
      ) : (
        <>
          {codeSent ? (
            <motion.div variants={fieldVariants} initial="hidden" animate="show" custom={1}>
              <p className="mb-2 text-[13px] leading-[18px] text-ink-500">
                Код отправлен на <span className="font-mono-num">{phone}</span>
              </p>
              <OtpInput onComplete={submitCode} />
              <button
                type="button"
                onClick={() => setCodeSent(false)}
                className="mt-3 text-[13px] font-semibold text-brand-600 transition-colors hover:text-brand-700"
              >
                Отправить код повторно
              </button>
            </motion.div>
          ) : (
            <motion.div variants={fieldVariants} initial="hidden" animate="show" custom={1}>
              <motion.button
                type="button"
                onClick={requestCode}
                whileTap={{ scale: 0.97 }}
                className="flex h-12 w-full items-center justify-center gap-2 rounded-xl bg-accent text-sm font-semibold text-white transition-all hover:-translate-y-px hover:bg-accent-hover"
              >
                {smsState === 'loading' ? (
                  <Loader2 size={18} className="animate-spin" />
                ) : (
                  'Получить код'
                )}
              </motion.button>
            </motion.div>
          )}
          {state === 'success' && (
            <p className="text-center text-sm font-semibold text-success">Код принят</p>
          )}
        </>
      )}

      <motion.div
        variants={fieldVariants}
        initial="hidden"
        animate="show"
        custom={4}
        className="flex items-center gap-3"
      >
        <span className="h-px flex-1 bg-brand-100" />
        <span className="text-[13px] text-ink-300">или</span>
        <span className="h-px flex-1 bg-brand-100" />
      </motion.div>

      <motion.div variants={fieldVariants} initial="hidden" animate="show" custom={5}>
        <motion.button
          type="button"
          onClick={() => {
            if (mode === 'password') {
              toast.message('Вход по SMS пока не настроен')
              return
            }
            setMode('password')
            setErrors({})
            setCodeSent(false)
          }}
          whileTap={{ scale: 0.97 }}
          className="flex h-12 w-full items-center justify-center rounded-xl border border-brand-100 bg-surface text-sm font-semibold text-ink-900 transition-colors hover:bg-brand-50"
        >
          {mode === 'password' ? 'Войти по коду из SMS' : 'Войти по паролю'}
        </motion.button>
      </motion.div>

      <motion.div
        variants={fieldVariants}
        initial="hidden"
        animate="show"
        custom={6}
        className="flex items-start gap-2.5 rounded-xl border-l-[3px] border-teal bg-info-bg p-3"
      >
        <Info size={18} className="mt-0.5 shrink-0 text-teal-dark" />
        <p className="text-sm leading-5 text-ink-900">
          Войдите тем же телефоном и паролем, которые указали при регистрации.
        </p>
      </motion.div>

      {directory.data && directory.data.length > 0 && (
        <div className="space-y-2">
          <p className="text-[13px] font-semibold text-ink-500">Войти как</p>
          <div className="grid gap-1.5">
            {directory.data.map((u) => (
              <button
                key={u.id}
                type="button"
                onClick={() => {
                  if (u.hasPassword) {
                    setPhone(formatPhone(phoneDigits(u.phone)))
                    toast.message('Введите пароль, который задали при регистрации')
                    return
                  }
                  void finish(u.id)
                }}
                className="flex items-center gap-3 rounded-xl border border-brand-100 bg-white px-3 py-2.5 text-left hover:bg-brand-50"
              >
                <img src={u.avatarUrl || '/avatar-1.png'} alt="" className="h-9 w-9 rounded-full object-cover" />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-semibold text-ink-900">{u.fullName}</span>
                  <span className="block truncate text-xs text-ink-500">
                    {u.position} · {u.phone}
                    {u.hasPassword ? ' · нужен пароль' : ''}
                  </span>
                </span>
              </button>
            ))}
          </div>
        </div>
      )}
    </form>
  )
}

/* ═══════════════ Таб «Регистрация» ═══════════════ */
const TIMEZONES = [
  'Калининград, UTC+2',
  'Москва, UTC+3',
  'Самара, UTC+4',
  'Екатеринбург, UTC+5',
  'Новосибирск, UTC+7',
  'Владивосток, UTC+10',
]

function RegisterTab() {
  const navigate = useNavigate()
  const utils = trpc.useUtils()
  const registerMut = trpc.auth.register.useMutation()
  const [name, setName] = useState('')
  const [phone, setPhone] = useState('')
  const [password, setPassword] = useState('')
  const [showPassword, setShowPassword] = useState(false)
  const [workspace, setWorkspace] = useState('')
  const [syncUrl, setSyncUrl] = useState('')
  const [timezone, setTimezone] = useState('Москва, UTC+3')
  const [agree, setAgree] = useState(false)
  const [state, setState] = useState<SubmitState>('idle')
  const [errors, setErrors] = useState<Record<string, string | undefined>>({})
  const [shakeTick, setShakeTick] = useState(0)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    if (state !== 'idle') return
    const next: Record<string, string | undefined> = {}
    if (!name.trim()) next.name = 'Введите имя и фамилию'
    if (phoneDigits(phone).length !== 11) next.phone = 'Введите телефон полностью'
    if (password.length < 12) next.password = 'Минимум 12 символов'
    if (!workspace.trim()) next.workspace = 'Введите название рабочего пространства'
    if (!agree) next.agree = 'Нужно согласие с условиями'
    setErrors(next)
    if (Object.keys(next).length > 0) {
      setShakeTick((t) => t + 1)
      return
    }
    try {
      setState('loading')
      await registerMut.mutateAsync({
        fullName: name.trim(),
        phone,
        password,
        workspaceName: workspace.trim(),
        timezone,
        syncUrl: syncUrl.trim() || undefined,
      })
      await utils.invalidate()
      setState('success')
      toast.success('Аккаунт создан')
      window.setTimeout(() => navigate('/'), 300)
    } catch (err) {
      setState('idle')
      toast.error(err instanceof Error ? err.message : 'Не удалось создать аккаунт')
    }
  }

  return (
    <form onSubmit={submit} noValidate className="space-y-4" key={`reg-${shakeTick}`}>
      <Field index={0} label="Имя и фамилия" error={errors.name} shake={!!errors.name}>
        <div className="relative">
          <UserRound
            size={18}
            className="pointer-events-none absolute left-4 top-1/2 -translate-y-1/2 text-ink-300"
          />
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Алексей Кузнецов"
            autoComplete="name"
            className={inputClass(!!errors.name)}
          />
        </div>
      </Field>

      <Field index={1} label="Телефон" required error={errors.phone} shake={!!errors.phone}>
        <div className="relative">
          <Phone
            size={18}
            className="pointer-events-none absolute left-4 top-1/2 -translate-y-1/2 text-ink-300"
          />
          <input
            type="tel"
            value={phone}
            onChange={(e) => setPhone(formatPhone(phoneDigits(e.target.value)))}
            placeholder="+7 (921) 555-01-42"
            autoComplete="tel"
            className={inputClass(!!errors.phone)}
          />
        </div>
      </Field>

      <Field index={2} label="Пароль" required error={errors.password} shake={!!errors.password}>
        <div className="relative">
          <Lock
            size={18}
            className="pointer-events-none absolute left-4 top-1/2 -translate-y-1/2 text-ink-300"
          />
          <input
            type={showPassword ? 'text' : 'password'}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="Придумайте пароль"
            autoComplete="new-password"
            className={cn(inputClass(!!errors.password), 'pr-11')}
          />
          <EyeToggle shown={showPassword} onToggle={() => setShowPassword((v) => !v)} />
        </div>
        <PasswordStrength password={password} />
      </Field>

      <Field
        index={3}
        label="Название рабочего пространства"
        required
        error={errors.workspace}
        shake={!!errors.workspace}
        hint={errors.workspace ? undefined : 'Компания или бригада. Сменить можно позже'}
      >
        <div className="relative">
          <Building2
            size={18}
            className="pointer-events-none absolute left-4 top-1/2 -translate-y-1/2 text-ink-300"
          />
          <input
            type="text"
            value={workspace}
            onChange={(e) => setWorkspace(e.target.value)}
            placeholder="ООО «СтройМонтаж»"
            className={inputClass(!!errors.workspace)}
          />
        </div>
      </Field>

      <Field index={4} label="Часовой пояс" shake={false}>
        <div className="relative">
          <Globe2
            size={18}
            className="pointer-events-none absolute left-4 top-1/2 -translate-y-1/2 text-ink-300"
          />
          <select
            value={timezone}
            onChange={(e) => setTimezone(e.target.value)}
            className={cn(inputClass(false), 'appearance-none pr-10')}
          >
            {TIMEZONES.map((tz) => (
              <option key={tz} value={tz}>
                {tz}
              </option>
            ))}
          </select>
          <ChevronDown
            size={16}
            className="pointer-events-none absolute right-4 top-1/2 -translate-y-1/2 text-ink-300"
          />
        </div>
      </Field>

      <motion.div variants={fieldVariants} initial="hidden" animate="show" custom={5}>
        <Checkbox
          checked={agree}
          onChange={setAgree}
          label={
            <>
              Соглашаюсь с{' '}
              <span className="font-semibold text-brand-600">условиями сервиса</span>
            </>
          }
        />
        {errors.agree && (
          <p className="mt-1 text-xs leading-4 text-danger">{errors.agree}</p>
        )}
      </motion.div>

      <motion.div variants={fieldVariants} initial="hidden" animate="show" custom={6}>
        <Field
          index={5}
          label="Дополнительный сервер (необязательно)"
          hint="Можно оставить пустым: этот телефон сам ведёт учёт и делится журналом по Wi‑Fi"
          shake={false}
        >
          <input
            value={syncUrl}
            onChange={(e) => setSyncUrl(e.target.value)}
            placeholder="https://sync.example.com"
            className={inputClass(false, false)}
          />
        </Field>

        <SubmitButton state={state}>Создать организацию</SubmitButton>
      </motion.div>
    </form>
  )
}

function JoinTab() {
  const navigate = useNavigate()
  const onCode = useCallback((value: string) => {
    const trimmed = value.trim()
    try {
      const parsed = JSON.parse(trimmed) as { t?: string; token?: string; server?: string }
      if (parsed.t === 'join' && parsed.token) {
        const peer = parsed.server ? `&peer=${encodeURIComponent(parsed.server)}` : ''
        navigate(`/join?token=${encodeURIComponent(parsed.token)}${peer}`)
        return
      }
    } catch {
      /* not json */
    }
    const joinMatch = trimmed.match(/[?&]token=([^&]+)/)
    const serverMatch = trimmed.match(/^https?:\/\/[^/?#]+/)
    if (joinMatch) {
      const peer = serverMatch ? `&peer=${encodeURIComponent(serverMatch[0])}` : ''
      navigate(`/join?token=${encodeURIComponent(decodeURIComponent(joinMatch[1]))}${peer}`)
      return
    }
    toast.error('Это не QR-приглашение в группу')
  }, [navigate])
  return (
    <div className="space-y-4">
      <p className="text-sm text-ink-500">
        Наведите камеру на QR администратора — вы сразу попадёте в его группу.
      </p>
      <QrScanner onCode={onCode} />
    </div>
  )
}

/* ═══════════════ Форма с табами «Вход / Регистрация» ═══════════════ */
export default function AuthForm() {
  const app = isNativeApp()
  const [params] = useSearchParams()
  const mode = params.get('mode')
  // На пустой базе первый владелец должен зарегистрироваться прямо в браузере:
  // сервер сам говорит, открыта ли сейчас регистрация.
  const optionsQ = trpc.auth.options.useQuery(undefined, { retry: 0 })
  const canRegister = app || optionsQ.data?.registrationOpen === true
  // Явный выбор пользователя; пока его нет — вкладка зависит от состояния базы.
  const [chosen, setChosen] = useState<'join' | 'login' | 'register' | null>(
    mode === 'register' ? 'register' : mode === 'login' ? 'login' : null,
  )
  const setTab = setChosen
  const fallbackTab = optionsQ.data?.bootstrap ? 'register' : 'join'
  const wantedTab = chosen ?? fallbackTab
  const tab = wantedTab === 'register' && !canRegister ? 'join' : wantedTab
  const tabs = canRegister
    ? [
        { id: 'join' as const, label: 'QR-код' },
        { id: 'login' as const, label: 'Вход' },
        { id: 'register' as const, label: 'Создать группу' },
      ]
    : [
        { id: 'join' as const, label: 'QR-код' },
        { id: 'login' as const, label: 'Вход' },
      ]

  return (
    <div className="w-full max-w-[400px]">
      <h1 className="text-[28px] leading-9 font-bold tracking-[-0.01em] text-ink-900">
        {tab === 'join' ? 'Присоединиться' : tab === 'login' ? 'Вход в Everyday' : 'Новая организация'}
      </h1>
      <p className="mt-1 text-sm text-ink-500">
        {app
          ? 'Этот телефон — узел учёта. Сервер не нужен: коллеги подключаются по QR в той же Wi‑Fi.'
          : optionsQ.data?.bootstrap
            ? 'База пустая: создайте организацию — вы станете её владельцем.'
            : canRegister
              ? 'Присоединяйтесь по QR, войдите или создайте новую организацию.'
              : 'Организация уже создана. Присоединиться можно по QR-приглашению администратора.'}
      </p>

      <div className="mt-6 flex border-b border-brand-100">
        {tabs.map((t) => (
          <button
            key={t.id}
            type="button"
            onClick={() => setTab(t.id)}
            className={cn(
              'relative flex-1 pb-3 text-sm font-semibold transition-colors',
              tab === t.id ? 'text-brand-600' : 'text-ink-500 hover:text-ink-900',
            )}
          >
            {t.label}
            {tab === t.id && (
              <motion.span
                layoutId="auth-tab-underline"
                transition={{ duration: 0.2, ease: EASE }}
                className="absolute inset-x-0 -bottom-px h-0.5 rounded-full bg-accent"
              />
            )}
          </button>
        ))}
      </div>

      {/* Cross-fade 200ms при переключении табов */}
      <div className="mt-6">
        <AnimatePresence mode="wait" initial={false}>
          <motion.div
            key={tab}
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.2 }}
          >
            {tab === 'join' ? <JoinTab /> : tab === 'login' ? <LoginTab /> : <RegisterTab />}
          </motion.div>
        </AnimatePresence>
      </div>
      {optionsQ.data?.androidTestBuild && !app && (
        <div className="mt-6 rounded-xl border border-brand-100 bg-brand-50/60 p-3 text-xs leading-5 text-ink-500">
          <a
            href={optionsQ.data.androidTestBuild.url}
            className="font-semibold text-brand-600 underline underline-offset-2"
            download
          >
            Скачать Android test APK
          </a>
          <div>{(optionsQ.data.androidTestBuild.sizeBytes / 1024 / 1024).toFixed(1)} МиБ · debug-сборка</div>
          <div className="break-all font-mono-num">SHA-256: {optionsQ.data.androidTestBuild.sha256}</div>
        </div>
      )}
      {optionsQ.data?.desktopTestBuild && !app && (
        <div className="mt-3 rounded-xl border border-brand-100 bg-brand-50/60 p-3 text-xs leading-5 text-ink-500">
          <a
            href={optionsQ.data.desktopTestBuild.url}
            className="font-semibold text-brand-600 underline underline-offset-2"
            download
          >
            Скачать автономную версию для Linux x86_64
          </a>
          <div>{(optionsQ.data.desktopTestBuild.sizeBytes / 1024 / 1024).toFixed(1)} МиБ · Rust-нода и offline UI</div>
          <div className="break-all font-mono-num">SHA-256: {optionsQ.data.desktopTestBuild.sha256}</div>
        </div>
      )}
    </div>
  )
}
