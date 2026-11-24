# Oeca Hop (web)

The phone side of Oeca Hop: an adaptation of LocalSend Web
(https://github.com/localsend/web, commit
`8fd3e7f6522f2ddc7c4cb0d237bb995ed95313c0`, Apache-2.0, see LICENSE). Its
signaling, WebRTC session and LocalSend messages are kept as they are;
Oeca adds the pieces below, marked `oeca:` in the code.

Naming: the product is Oeca Hop (it was Oeca Nearby). The code, the folder
`nearby-web/`, the origin `https://nearby.oeca.app`, storage keys
(`oeca-nearby-*`, the `oeca-nearby` database), the protocol name (Oeca Nearby
Verification v1, `x-oeca-nearby-*` SDP lines) and the native identity's
derivation keep the old name, since changing them would break stored
identities, trust records and compatibility with Oeca. Below, "Nearby" means
this code. The site's icons (`favicon.ico`, `apple-touch-icon.png`,
`logo-512.png`) are Oeca Hop's own mark, no longer LocalSend's logo.

| File | Change |
|---|---|
| `app/services/trust.ts` (new) | Persistent DTLS certificate (IndexedDB), the verification extension's construction and code (same algorithm as Oeca's `src/nearby.rs`), strict fingerprint parsing. |
| `app/services/proof.ts` (new) | Claimed, connected and authenticated identities as separate types; the exchange on the extension channel (as `src/nearby/proof.rs`). |
| `app/services/devices.ts` (new) | Trusted and revoked devices, and the attempt limit, in IndexedDB; the device-list API (as `src/nearby/devices.rs`). |
| `app/services/transfer.ts` (new) | The state machine every decision goes through, by transfer id (as `src/nearby/transfer.rs`). |
| `app/services/limits.ts` (new) | The bounds on what a peer can make the page hold or do; file-list validation; safe file names and dangerous types (see File names). |
| `app/services/testing.ts` (new) | Synthetic identities and fakes for the tests. |
| `app/services/webrtc.ts` | No PIN either way (see No PIN); uses the persistent certificate; verification gate before the file list is sent; real `DECLINED` when nothing is selected; incoming chunks go to a sink, not a Blob; only selected files with their tokens are accepted; sizes checked; cancel (`AbortSignal`), answer / connect timeouts, and every exit closes the connection. |
| `app/utils/fileSink.ts`, `app/workers/sink.worker.ts` (new) | Streaming receive into the origin-private file system (sync access handle in a worker); in-memory fallback capped at 256 MB; storage-quota check before accepting. |
| `app/services/store.ts` | Wires the session to `transfer.ts`: decisions by transfer id, 120 s decision timeout, file-count / size limits, one session at a time, reconnect on `online` and after 30 s in the background, screen wake lock during transfers. |
| `app/utils/streamController.ts` | `close()` / `error()` so reads end when the connection does; an optional bound on what is queued and unread. |
| `app/services/signaling.ts` | `close()` (to reconnect a stale socket); an answer counts only from the peer the offer went to; a frame that is no valid message is dropped. |
| `app/pages/index.vue`, `app/components/dialog/SessionDialog.vue`, `Dialog.vue`, `FileProgress.vue`, `ProgressBar.vue`, `ProgressRing.vue`, `FileStack.vue`, `Radar.vue`, `IdentityCard.vue`, `Notch.vue`, `SheetHeader.vue` (new) | The Oeca Nearby interface: a privacy step before anything connects (remembered per notice version), the connection states, the device list, one sheet per transfer (identity and its warnings, the code, every offered file, confirmed progress, each file's outcome, Save); decision buttons dim until the question arms; Accept only where the trust layer allows it. |
| `app/components/SendSheet.vue`, `SettingsSheet.vue`, `PrivacyNotice.vue`, `PrivacySheet.vue`, `ConfirmDialog.vue`, `Glyph.vue`, `app/utils/confirm.ts` (new) | Sending (the destination, the files picked, removable, then Send); Settings as pages in one sheet (this browser's name, appearance, trusted devices with Forget / Retry / Forget all, identity reset, the privacy notice, turning Nearby off); confirmations for what can't be undone (Cancel focused, the destructive action red); the interface icons: Phosphor Icons 2.1 (MIT, `@phosphor-icons/core`, a dev dependency), Bold and Fill, bundled as static SVG through `app/utils/glyphs.ts` (only the icons used; named in the notices). |
| `app/utils/view.ts` (new) | What the page shows, as tested functions: status, identity badges (never from a name), stage titles, per-file outcomes, confirmed progress (sending counts a file once the receiver confirmed it), send checks, the privacy notice and its id. |
| `app/assets/css/tailwind.css` (new), `tailwind.config.ts` | Light and dark color tokens (the system's choice or Settings'), touch-size buttons, a progress style without inline styles, Reduced Motion. |
| `endpoints.ts` (new), `vitest.config.ts` (new) | Public signaling and STUN only in a production build unless set (see below); the tests resolve `~` / `@` without Nuxt. |
| `nuxt.config.ts`, `i18n/locales/en.json` | Names (Oeca Hop), base URL `https://nearby.oeca.app`, no DevTools, no inline styles, the policy on every page, `STUN_SERVERS`; no service-worker module. |
| `notices.ts` (new) | The site's `THIRD-PARTY-NOTICES.txt`, generated at each build from the packages in the bundle (see Notices). |
| `csp.ts` (new) | The Content-Security-Policy and security headers (see Security headers). |
| `app/components/FileName.vue` (new) | A file's name with its extension always shown, and a mark on types that can run programs. |
| `public/fonts/` (new), `nuxt.config.ts` | General Sans (Oeca's typeface, ITF Free Font License) is the site's face: the build copies it from `../assets/fonts`, where it is git-ignored because the license does not allow publishing the files in this repository, and serves it at `/fonts/general-sans-*.otf`, named in the notices. A build without it falls back to Hubot Sans (SIL Open Font License 1.1, Copyright 2023 The Hubot Sans Project Authors; bundled, license as `OFL.txt`) and the system face. No remote fonts (`font-src 'self'`). |
| `public/manifest.webmanifest`, `public/sw.js` (new) | A static manifest (installable without a service worker); `sw.js` only removes a stale worker (see Updates). |
| `app/layouts/default.vue`, `app/app.vue` | No inline style attributes or injected `<style>` (the policy refuses them). The theme and language switchers were removed: the theme is in Settings, and the interface is in English. |
| `i18n/locales/*.json` | The PIN strings removed. |

Stage 2.1: the browser keeps its DTLS certificate until it expires (Chromium
caps it at 365 days; checked), then makes a new one and says so; a stable
alias (localStorage) so peers can tell "same name, other certificate"; trust
can be revoked ("Forget this device", state "revoked"); cancel closes the
data channel first so the peer stops at once; a peer's channel close ends the
session as "stopped by the other device".

## Verification (Option B, strict)

An Oeca-only extension, described at `OSV` in `app/services/trust.ts`: a
commitment in the offer's SDP, a random in the answer's, and a reveal on the
data channel `oeca-nearby-v1`, opened only when both sides took part. The code
covers both randoms and both DTLS fingerprints of the connection. LocalSend's
messages are unchanged.

On that channel each side also states its room id, which must be the peer the
other side meant, and the answerer acknowledges the reveal (`proof.ts` has the
messages). Both ends need this version: without those fields authentication
fails, it does not fall back.

- Identity (new, trusted, changed, revoked) and capability (stock, Oeca) are
  separate. A fingerprint in an SDP is a claim. A code and a trusted name
  exist only for an authenticated connection.
- A trusted device is a certificate fingerprint under the name the user
  confirmed. It is never renamed or replaced because of an alias; another
  alias asks the user.
- A stock LocalSend peer has no code, is never remembered and gets one
  transfer at a time on an explicit choice, only while it is unknown. A known
  (trusted, changed, revoked) device without the extension is refused.
- A busy Oeca answers `a=x-oeca-nearby-busy:1`: no random, no connection.
- Every decision names its transfer's id, is valid only in the stage that
  asks, and only 700 ms after that question was on screen.
- Revocation is effective before it is saved and is reported as done only
  once saved; until then all trust is paused. Revoked records are never
  dropped (256 trusted, 4096 records in all; at a limit, new pins fail).
  Clearing the site's data clears them: then nobody is trusted.
- An SDP with differing or malformed fingerprint lines is refused; where the
  browser exposes the DTLS certificate it must match the SDP's.
- Handshakes answered per 10 minutes: 4 per peer, 10 in all, across tabs and
  reloads.

## Bounds

- SDP: 128 KiB as signaled, 64 KiB inflated (inflating stops there). One
  signaling message: 1 MiB.
- A control message (file list, token list): 1 MiB and 4096 chunks, while it
  arrives. Queued and unread: 2 MiB until accepted, 64 MiB while files arrive
  (a data channel cannot be paused). 4 channels per session.
- File list: up to 1000 entries parsed (100 offered to the user), unique
  string ids up to 128, names up to 1024 bytes of UTF-8, safe-integer sizes.
  One approval is each file once, at its declared size.
- Every stage has a deadline. During a transfer it restarts only after 64 KiB
  moved (30 s receiving, 120 s sending).
- While busy: at most 2 connections made only to decline and 30 answers per
  10 minutes (5 per peer); beyond that, no answer. Received files that are not
  saved yet count as busy, so a new offer cannot discard them.
- The user's own send replaces an incoming connection that proved nothing
  and asked nothing.

## Status and reconnecting

- Sent: the receiver confirmed each file, by id, as saved. Received: every
  approved file arrived. Declined: only the protocol's DECLINED. Anything else
  is a failure.
- Reconnecting backs off with jitter; only a connection that lasted 60 s
  resets it. Attempts are 10 s apart at least when the network or tab comes
  back, and the server's 429 is a 15-minute wait that nothing shortens.
- A connected transfer does not need signaling and outlives its loss.

`i18n/locales/en.json` is also modified (the title); JSON has no comments, so
this file is its change notice.

Default STUN is LocalSend's (`stun.localsend.org`), as on the native side;
`STUN_SERVERS` at build time replaces it (`STUN_SERVERS=""`: none, for
loopback test builds). Only a production build (`nuxt generate`) uses the
public signaling and STUN servers by default: the dev server and anything else
use `ws://127.0.0.1:9/` (nothing listens) and no STUN, unless `SIGNALING_URL` /
`STUN_SERVERS` say otherwise (`endpoints.ts`).

## Build and host

```
corepack pnpm install --frozen-lockfile   # pnpm 12.21.0 (packageManager); esbuild's install script is refused (pnpm-workspace.yaml)
corepack pnpm vitest run
corepack pnpm nuxt generate              # static site in .output/public
```

Never run the dev server on an untrusted network. Any static HTTPS host works
for the output; HTTPS is required (WebCrypto, WebRTC certificates and OPFS
need a secure context). Signaling is `wss://public.localsend.org/v1/ws`
(`SIGNALING_URL` at build time; the policy's `connect-src` follows it).

## Origin

Nearby is served from its own origin, `https://nearby.oeca.app`, never from
`oeca.app` or a path under it. Everything Nearby trusts is bound to the origin:

| Record | Where | What it holds |
|---|---|---|
| `oeca-nearby` / `identity` | IndexedDB | This browser's DTLS certificate (its private key cannot be exported) |
| `oeca-nearby-trust` / `state` | IndexedDB | Trusted and revoked devices, the attempt limit |
| `oeca-nearby-alias` | localStorage | The stable alias |
| `oeca-nearby-trust-hold` | Web Locks | Serializes trust writes between tabs |
| `oeca-incoming` | Origin-private file system | Received files not yet saved |

Any script on the same origin could use the identity and rewrite trust, so
the website (or anything else) must never share it. Nearby was never
published before this origin was chosen, so no browser holds Nearby data on
another origin and nothing is migrated. Should that ever be needed, nothing
is copied across origins: the certificate cannot leave its origin, and trust
must not arrive by an unauthenticated channel. Devices verify the new
identity once, as after cleared site data, and the page says so.

Deployment requirements (not done here): DNS for `nearby.oeca.app` to a
static host serving `.output/public` over HTTPS only, nothing else on that
host name, and the headers below.

## Security headers

Every page carries its policy in a `<meta>` tag, and `_headers` (read by
Netlify and Cloudflare Pages; on another host, configure the same headers
for every path) repeats it with what only a header can say:

- `Content-Security-Policy`: `default-src 'none'`; scripts from this origin
  plus the two inline scripts Nuxt writes (the import map and its config),
  allowed by hash only; styles, images, fonts, workers and the manifest
  from this origin; connections to this origin and the signaling server only;
  `base-uri`, `form-action`, `object-src` none; `frame-ancestors 'none'`
  (header only). No `unsafe-inline`, `unsafe-eval`, wildcards, `data:` or
  `blob:`. WebRTC itself (ICE, STUN, the data channel) is not governed by
  CSP.
- `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
  `Cross-Origin-Opener-Policy: same-origin`, a `Permissions-Policy` that
  denies camera, microphone, location, payment, USB, serial and Bluetooth.
- `Cache-Control: no-cache` everywhere: each load asks the host (by ETag),
  so a fix reaches every visitor on their next load.

Without the headers, the page still refuses to run inside a frame (it says
so and connects nowhere).

## File names

Every received name is rewritten once, when the list arrives
(`limits.ts` `safeName`, the same rules and test vectors as Oeca's
`src/nearby.rs` `safe_name`): no control, bidi or invisible characters, so
the name shown is the name saved and its real extension is its last; no
`<>:"/\|?*` (no paths, no streams); no Windows device names (`CON`,
`CONIN$`, `COM¹`...); no trailing dots or spaces; NFC; at most 235 UTF-16
units and 255 bytes, shortened before the extension. On screen a long name
is shortened before its extension, which always shows, and a type that can
run programs or open another place (`.exe`, `.msi`, `.lnk`, `.url`,
`.library-ms`, `.ps1`...) is marked. Nothing is opened: files are saved by
a tap on Save.

The page cannot write Windows' Mark of the Web: the browser does. Chrome on
Windows marks a saved file as downloaded from the internet (ZoneId 3, with
the page's address; checked), and refuses to save `.url` under that name.
Phones have no such mark.

The file list this page sends carries names, sizes and types, no dates.

## Identity reset

`store.ts` `resetIdentity(forgetDevices)` deletes this browser's
certificate and alias (only those): the next visit makes new ones, and
devices that trusted it ask to verify it again (they cannot be told). The
devices trusted here are kept, or with `forgetDevices` each is revoked (as
"Forget"). Refused while a transfer runs. Its control comes with the
Nearby UI; clearing the site's data does the same with `forgetDevices`.

## Logs and randomness

Nothing a page logs identifies a device or a transfer: no SDP, addresses,
tokens, fingerprints, keys or file names (upstream logged all of them). What
is left are a few warnings with fixed text, and a build drops any
`console.log`, `info`, `debug` or `trace` a later change adds
(`nuxt.config.ts`). Session ids, file tokens, transfer ids and the
verification secrets come from `crypto.getRandomValues` (256 bits); without
it, nothing is sent. `Math.random` is left only for the reconnect jitter and
the random alias (a name, never trusted). The signing key cannot be exported.

## Tabs

- Identity: a new certificate is stored only if none is, in one IndexedDB
  transaction (the browser runs one tab's at a time) that must commit: two
  tabs opened together end with one identity, and a write that did not
  commit is never taken as kept.
- Trust: every change is one transaction over the whole record, and a
  revocation reaches the other tabs at once (BroadcastChannel), which end a
  live transfer with that device.
- Received files: each tab writes in its own OPFS folder and holds a Web
  Lock on it while it lives. "Done" clears only its own folder; a starting
  tab removes the folders of tabs that are gone (closed, crashed). Without
  Web Locks no other tab's folder is touched.
- The alias: two tabs opened together on a first visit may show different
  random names until the next load (localStorage keeps one). Cosmetic: the
  alias is never trusted.

## Notices

Each build writes `THIRD-PARTY-NOTICES.txt` into the site, from the packages
whose code the bundle contains (Vite's module list) plus Tailwind (its CSS),
with each package's own license and notice files, the fork's Apache-2.0
license and the Material Symbols attribution. No notices, no site: the build
fails without them.

## No PIN

The page neither asks for a PIN nor demands one. A peer's `PIN_REQUIRED` (as
a receiver's token answer, or a sender's list answer) ends the transfer with
"The other device asks for a PIN…": no dialog, nothing typed, nothing sent.
Any peer can send it, and what a user typed would go to that peer; Oeca's
verification code is what identifies a device.

## Updates (no service worker)

Nearby has no service worker: it cannot work offline (it needs signaling),
and a cached copy could keep outdated code running. Updates are ordinary
loads. On load, the page removes any worker registered on its origin (an
earlier build's). A worker that answers navigations itself would keep the
page from loading: for that case `sw.js`, served at the address such a
worker used, replaces it, deletes Cache Storage and reloads the page. Site
data (identity, trust) is never touched by updates.

## Browser limits (stated in the UI)

- Large files stream to the site's storage (OPFS); the ceiling is the storage
  quota the browser grants, checked before accepting. Without OPFS sync
  access (older Safari, some private modes) files are held in memory, up to
  256 MB.
- Received files are saved by a tap on Save (iOS Safari requires a gesture).
- A background tab or a locked phone can suspend the page and end a transfer;
  the screen is kept on during transfers where the Wake Lock API exists.
- Private browsing may not keep the page's identity: Oeca then asks to verify
  again next visit.
