# Oeca website

The product site for Oeca: a single static page built with Next.js (App Router, static export), TypeScript, Tailwind CSS 4 and GSAP. It is independent of the Rust app, its build and its installer.

## Develop

Requires Node.js 20.9 or later.

```powershell
cd website
npm ci
npm run dev        # http://localhost:3000
```

```powershell
npm run lint
npm run typecheck
npm run build      # static site in website/out
```

## Deploy on Vercel (free)

1. In Vercel, choose **Add New → Project** and import the `ujvall/Oeca` GitHub repository.
2. Set **Root Directory** to `website`.
3. Leave the detected preset (**Next.js**) and the default build settings (`npm run build`). No environment variables are needed.
4. Deploy. The site is served from the project's `*.vercel.app` domain. Open Graph URLs use Vercel's production domain automatically (`VERCEL_PROJECT_PRODUCTION_URL`).

Any static host also works: deploy the contents of `out/` after `npm run build`.

## Content

- `lib/releases.ts` lists the published GitHub Releases, newest first, and drives both the `/changelog` page and the home page's version and download button. To add a release, copy its tag, publish date (UTC), release URL and installer asset URL from GitHub, keep the notes' wording under Added / Improved / Fixed / Changed, and put the entry at the top. Leave out `installerUrl` if the release has no installer; leave `sections` empty if it has no notes.
- `lib/site.ts` holds the repository links and requirements. `CAPTURED_VERSION` is the build the screenshots come from; change it only when the captures are retaken.
- `public/media/` holds the captures. Each one was taken from a running Oeca 0.1.0 at 160% scale with exact transparency, using an example media session ("Example track", "Sample artist") and default settings. `space-clipboard.webp` comes from the repository's `assets/readme/clipboard.png`, with its wallpaper removed. The icons are resized from `assets/branding/Oeca.png`.
- General Sans is loaded from Fontshare's CSS API under the ITF Free Font License. No font files are kept in this repository.

## Motion

GSAP (`lib/gsap.ts`) runs one hero sequence, the capture-to-capture morph in the spaces showcase (also used when the hero notch is clicked), a scrubbed push-in on the closed-notch panel, the download icon's entrance, and the changelog's timeline: a pill on the left edge showing the version you're reading, which widens to the right into the full release list and closes back to the left. Every tween starts from the value on screen, so it can be interrupted. With `prefers-reduced-motion: reduce`, changes are instant and nothing is hidden. Without JavaScript, the page renders fully.
