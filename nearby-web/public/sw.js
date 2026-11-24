// oeca: Nearby has no service worker, and the page never registers this one.
// It is here only for a browser that still holds a worker registered at this
// address (an earlier build's): the browser's update check fetches it, and it
// removes that registration and its caches, then reloads open pages onto the
// current version. Site data (identity, trusted devices) is never touched.
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      for (const key of await caches.keys()) await caches.delete(key);
      await self.registration.unregister();
      for (const client of await self.clients.matchAll({ type: "window" })) client.navigate(client.url);
    })(),
  );
});
