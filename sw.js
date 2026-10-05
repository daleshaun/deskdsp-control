// Minimal service worker: present so the page is installable as a PWA.
// Network-first (the app is live control; nothing useful to serve offline).
self.addEventListener("install", (e) => { self.skipWaiting(); });
self.addEventListener("activate", (e) => { self.clients.claim(); });
self.addEventListener("fetch", (e) => {
  e.respondWith(fetch(e.request).catch(() => caches.match(e.request)));
});
