import { Toaster } from 'sonner'
import BrandPanel from '@/components/auth/BrandPanel'
import AuthForm from '@/components/auth/AuthForm'
import MeshCanvas from '@/components/auth/MeshCanvas'

/**
 * Страница входа / регистрации (маршрут /login, без Layout).
 * auth.md: десктоп — двухколоночный экран 100dvh (55% бренд-панель / 45% форма);
 * мобайл — хедер 160px с логотипом на mesh-фоне + белая область формы
 * со скруглением 18px сверху (нахлест -18px).
 */
export default function Auth() {
  return (
    <div className="min-h-[100dvh] bg-surface lg:flex">
      {/* Бренд-панель — десктоп (55%) */}
      <div className="hidden lg:block lg:w-[55%]">
        <div className="sticky top-0 h-[100dvh]">
          <BrandPanel />
        </div>
      </div>

      {/* Мобайл: компактный хедер 160px с логотипом на тёмном mesh-фоне */}
      <div className="relative h-40 overflow-hidden bg-[#2E3160] bg-grad-mesh lg:hidden">
        <MeshCanvas />
        <div className="relative z-10 flex h-full items-start p-6">
          <img src="/logo.svg" alt="Everyday" className="h-9 w-auto" />
        </div>
      </div>

      {/* Правая панель — форма (45% на десктопе); на мобайле — нахлест -18px */}
      <div className="relative z-10 -mt-[18px] flex-1 rounded-t-[18px] bg-surface lg:mt-0 lg:w-[45%] lg:flex-none lg:rounded-none">
        <div className="flex min-h-[calc(100dvh-142px)] items-center justify-center px-6 py-10 lg:min-h-[100dvh] lg:px-12">
          <AuthForm />
        </div>
      </div>

      {/* Тосты: снизу по центру, pill, тёмный ink-900 фон (design.md §6) */}
      <Toaster
        position="bottom-center"
        toastOptions={{
          style: {
            background: '#303466',
            color: '#fff',
            border: 'none',
            borderRadius: '999px',
          },
        }}
      />
    </div>
  )
}
