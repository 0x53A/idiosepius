importScripts("./pkg/cache-manifest.js");
const CACHE_PREFIX = `idiosepius-offline-${encodeURIComponent(self.registration.scope)}-`;
const CACHE_NAME = `${CACHE_PREFIX}${self.IDIOSEPIUS_OFFLINE.version}`;
// Keep this handle for the worker's lifetime. An old in-flight request must
// not reopen and recreate its cache after a newer worker has removed it.
const currentCache = caches.open(CACHE_NAME);

self.addEventListener("install", (event) => {
  event.waitUntil(
    (async () => {
      const cache = await currentCache;
      try {
        await cache.addAll(
          self.IDIOSEPIUS_OFFLINE.assets.map((url) => new Request(url, { cache: "reload" })),
        );
      } catch (error) {
        await caches.delete(CACHE_NAME);
        throw error;
      }
      await self.skipWaiting();
    })(),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((names) =>
        Promise.all(
          names
            .filter(
              (name) => name.startsWith(CACHE_PREFIX) && name !== CACHE_NAME,
            )
            .map((name) => caches.delete(name)),
        ),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (event) => {
  const { request } = event;
  const url = new URL(request.url);

  if (request.method !== "GET" || url.origin !== self.location.origin) {
    return;
  }

  const network = fetch(request);
  event.waitUntil(
    network
      .then((response) => {
        if (!response.ok || response.type !== "basic") {
          return;
        }
        return currentCache
          .then((cache) => cache.put(request, response.clone()));
      })
      .catch(() => {}),
  );

  event.respondWith(
    network.catch(async () => {
      const cache = await currentCache;
      const cached = await cache.match(request);
      if (cached) {
        return cached;
      }
      if (request.mode === "navigate") {
        return cache.match("./index.html");
      }
      return Response.error();
    }),
  );
});
