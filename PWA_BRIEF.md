# Brief: make the tablet page installable as a home-screen app (PWA)

The page can "live on the tablet" as an installed app (own icon, full-screen, no
browser chrome) while the DSP engine stays on the Mac. This needs 4 static files
served by the remote server. All are in the repo root already:
`tablet_v3.html` (updated), `manifest.webmanifest`, `sw.js`,
`pwa-icon-192.png`, `pwa-icon-512.png`.

## 1. Re-bake the page
Replace `TABLET_TOUCH_HTML` in `src/remote/assets.rs` with the updated
`tablet_v3.html` (it now links the manifest, sets theme-color/apple-touch-icon,
remembers the token in localStorage, and registers `sw.js`).

## 2. Serve the PWA files (ungated — no token) from `src/remote/mod.rs`
Add these routes to the router (public, like `/`):
- `GET /manifest.webmanifest` -> body = contents of `manifest.webmanifest`,
  Content-Type `application/manifest+json`.
- `GET /sw.js` -> body = contents of `sw.js`, Content-Type `text/javascript`.
  (Serve it from the ROOT path so its scope covers `/`.)
- `GET /icon-192.png` -> bytes of `pwa-icon-192.png`, Content-Type `image/png`.
- `GET /icon-512.png` -> bytes of `pwa-icon-512.png`, Content-Type `image/png`.

Embed them with `include_str!` / `include_bytes!` so the binary stays
self-contained, e.g.:
```rust
const MANIFEST: &str = include_str!("../../manifest.webmanifest");
const SW_JS: &str   = include_str!("../../sw.js");
const ICON_192: &[u8] = include_bytes!("../../pwa-icon-192.png");
const ICON_512: &[u8] = include_bytes!("../../pwa-icon-512.png");
```
Return them with the right `content-type` header (use `axum::response::Response`
or `([(header::CONTENT_TYPE, "...")], BODY)`).

Do NOT token-gate these four routes — the browser fetches them without the
token, and they contain nothing sensitive. Leave `/ws` and `/api/*` gated as-is.

## 3. Install on the tablet
- Keep using the USB bridge (`adb reverse`) and open
  `http://localhost:8080/?token=studio2026` — **localhost is a secure context**,
  so the service worker registers and Chrome offers "Install app" / "Add to Home
  screen". After installing, the icon launches full-screen; the saved token lets
  it reconnect without the query string.
- Note: over plain WiFi (`http://<ip>:8080`, not localhost) it is NOT a secure
  context, so the SW won't register and install won't be offered — the page still
  works as a normal site. (Install via the USB/localhost path.)

## Done =
Opening over localhost offers "Install"; the installed app launches standalone,
remembers the token, and drives the Zen Go exactly like the browser page. Push to
main; hand back for review.
