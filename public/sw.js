const CACHE='bit-community-v2',FILES=['/','/index.html','/style.css','/app.js','/manifest.json'];
self.addEventListener('install',e=>e.waitUntil(caches.open(CACHE).then(c=>c.addAll(FILES))));
self.addEventListener('fetch',e=>{if(new URL(e.request.url).pathname.startsWith('/api/'))return;e.respondWith(fetch(e.request).then(r=>{const x=r.clone();caches.open(CACHE).then(c=>c.put(e.request,x));return r}).catch(()=>caches.match(e.request)))});
