/* global importScripts */
importScripts('/precache-manifest.js')

const precache = self.__EVERYDAY_PRECACHE__ || { version: 'development', files: [] }
const CACHE_PREFIX = 'everyday-shell-'
const CACHE = `${CACHE_PREFIX}${precache.version}`
const OFFLINE_PAGE = '/offline.html'
const PRECACHED_PATHS = new Set(precache.files.map((file) => new URL(file, self.location.origin).pathname))
const LEGACY_CACHES = new Set(['meshkeeper-v2'])

self.addEventListener('install', (event) => {
  event.waitUntil(caches.open(CACHE).then((cache) => cache.addAll(precache.files)))
})

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((key) => (key.startsWith(CACHE_PREFIX) && key !== CACHE) || LEGACY_CACHES.has(key)).map((key) => caches.delete(key))))
      .then(() => self.clients.claim()),
  )
})

self.addEventListener('message', (event) => {
  if (event.data?.type === 'SKIP_WAITING') self.skipWaiting()
})

async function networkFirst(request) {
  try {
    return await fetch(request)
  } catch {
    return (await caches.match(request)) || (await caches.match('/index.html')) || (await caches.match(OFFLINE_PAGE))
  }
}

async function cacheFirst(request) {
  const cached = await caches.match(request)
  if (cached) return cached
  const response = await fetch(request)
  if (response.ok) {
    const cache = await caches.open(CACHE)
    await cache.put(request, response.clone())
  }
  return response
}

self.addEventListener('fetch', (event) => {
  const request = event.request
  if (request.method !== 'GET') return

  const url = new URL(request.url)
  if (url.origin !== self.location.origin) return
  if (url.pathname.startsWith('/api') || url.pathname.startsWith('/sync') || url.pathname === '/health') return

  if (request.mode === 'navigate') {
    event.respondWith(networkFirst(request))
    return
  }
  if (PRECACHED_PATHS.has(url.pathname)) event.respondWith(cacheFirst(request))
})
