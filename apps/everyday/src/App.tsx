import { Routes, Route } from 'react-router'
import { lazy, Suspense } from 'react'
import { StoreProvider } from '@/lib/store'
import Layout from '@/components/Layout'
import PwaStatus from '@/components/PwaStatus'
import { Toaster } from 'sonner'

const Catalog = lazy(() => import('@/pages/Catalog'))
const MyTools = lazy(() => import('@/pages/MyTools'))
const ToolCard = lazy(() => import('@/pages/ToolCard'))
const CreateTool = lazy(() => import('@/pages/CreateTool'))
const Transfers = lazy(() => import('@/pages/Transfers'))
const History = lazy(() => import('@/pages/History'))
const Inventory = lazy(() => import('@/pages/Inventory'))
const Notifications = lazy(() => import('@/pages/Notifications'))
const Reports = lazy(() => import('@/pages/Reports'))
const Admin = lazy(() => import('@/pages/Admin'))
const Profile = lazy(() => import('@/pages/Profile'))
const Auth = lazy(() => import('@/pages/Auth'))
const Scan = lazy(() => import('@/pages/Scan'))
const Join = lazy(() => import('@/pages/Join'))
const Chat = lazy(() => import('@/pages/Chat'))
const Invite = lazy(() => import('@/pages/Invite'))
const Knowledge = lazy(() => import('@/pages/Knowledge'))
const BitWallet = lazy(() => import('@/pages/BitWallet'))

function PageFallback() {
  return (
    <div className="flex min-h-40 items-center justify-center text-sm text-ink-500" role="status">
      Загрузка…
    </div>
  )
}

export default function App() {
  return (
    <StoreProvider>
      <PwaStatus />
      <Toaster position="bottom-center" richColors />
      <Suspense fallback={<PageFallback />}>
        <Routes>
          <Route path="/login" element={<Auth />} />
          <Route path="/join" element={<Join />} />
          <Route element={<Layout />}>
            <Route index element={<Catalog />} />
            <Route path="scan" element={<Scan />} />
            <Route path="my" element={<MyTools />} />
            <Route path="tool/:id" element={<ToolCard />} />
            <Route path="create" element={<CreateTool />} />
            <Route path="transfers" element={<Transfers />} />
            <Route path="history" element={<History />} />
            <Route path="inventory" element={<Inventory />} />
            <Route path="notifications" element={<Notifications />} />
            <Route path="chat" element={<Chat />} />
            <Route path="knowledge" element={<Knowledge />} />
            <Route path="bit" element={<BitWallet />} />
            <Route path="invite" element={<Invite />} />
            <Route path="reports" element={<Reports />} />
            <Route path="admin" element={<Admin />} />
            <Route path="profile" element={<Profile />} />
            <Route path="*" element={<Catalog />} />
          </Route>
        </Routes>
      </Suspense>
    </StoreProvider>
  )
}
