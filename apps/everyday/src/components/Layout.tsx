import { Navigate, Outlet } from 'react-router'
import Sidebar from '@/components/Sidebar'
import TopBar from '@/components/TopBar'
import MobileNav from '@/components/MobileNav'
import { trpc } from '@/providers/trpc'

export default function Layout() {
  const meQ = trpc.auth.me.useQuery(undefined, { retry: 0 })
  if (!meQ.data && !meQ.isLoading) {
    return <Navigate to="/login" replace />
  }

  return (
    <div className="min-h-[100dvh] bg-app flex">
      <Sidebar />
      <div className="flex-1 min-w-0 flex flex-col">
        <TopBar />
        <MobileNav />
        <main className="flex-1 pb-24 lg:pb-8">
          <div className="mx-auto max-w-container px-3 sm:px-4 lg:px-6 py-4 lg:py-6">
            <Outlet />
          </div>
        </main>
      </div>
    </div>
  )
}
