const CACHE_PREFIX = "idiosepius-offline-";
const CACHE_NAME = `${CACHE_PREFIX}v1`;
const PACKAGE_MANIFEST = "./pkg/asset-manifest.json";
const APP_SHELL = [
  "./",
  "./index.html",
  "./bootstrap.js",
  "./manifest.webmanifest",
  "./icons/icon-192.png",
  "./icons/icon-512.png",
  "./icons/icon-maskable-512.png",
  "./icons/apple-touch-icon.png",
];

self.addEventListener("install", (event) => {
  event.waitUntil(
    (async () => {
      const response = await fetch(PACKAGE_MANIFEST);
      if (!response.ok) {
        throw new Error("Could not read the web package manifest");
      }
      const packageAssets = await response.json();
      const cache = await caches.open(CACHE_NAME);
      await cache.addAll([...APP_SHELL, PACKAGE_MANIFEST, ...packageAssets]);
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
        return caches
          .open(CACHE_NAME)
          .then((cache) => cache.put(request, response.clone()));
      })
      .catch(() => {}),
  );

  event.respondWith(
    network.catch(async () => {
      const cached = await caches.match(request);
      if (cached) {
        return cached;
      }
      if (request.mode === "navigate") {
        return caches.match("./index.html");
      }
      return Response.error();
    }),
  );
});
