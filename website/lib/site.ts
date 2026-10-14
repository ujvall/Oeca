import { LATEST } from "./releases";

// Facts shown on the page. Each one is checked against the repository:
// version and installer = the newest entry in releases.ts (GitHub Releases);
// requirements = installer/Oeca.iss (MinVersion=10.0, x64compatible,
// PrivilegesRequired=lowest). A new release is added in releases.ts only.
export const VERSION = LATEST.version;
/** The build the screenshots in public/media were taken from (not the latest). */
export const CAPTURED_VERSION = "0.1.0";

export const REPO_URL = "https://github.com/ujvall/Oeca";
export const RELEASES_URL = `${REPO_URL}/releases`;
export const DOWNLOAD_URL = LATEST.installerUrl ?? `${REPO_URL}/releases`;
export const LICENSE_URL = `${REPO_URL}/blob/master/LICENSE`;
export const REQUIREMENTS = "Windows 10 or later, 64-bit";

// Absolute base for Open Graph URLs: Vercel sets the production domain at build time.
export const SITE_URL = process.env.VERCEL_PROJECT_PRODUCTION_URL
  ? `https://${process.env.VERCEL_PROJECT_PRODUCTION_URL}`
  : "http://localhost:3000";

/** A real capture of the running app. Sizes are CSS pixels; `body` is the
 *  opaque notch inside the image (left, top, right, bottom), without shadow. */
export type Shot = {
  src: string;
  alt: string;
  w: number;
  h: number;
  body: [number, number, number, number];
  /** Bottom corner radius of the notch body, CSS px. */
  radius: number;
};

// Captured at 160% scale, so 1 CSS px = 1.6 image px.
const at160 = (
  src: string,
  alt: string,
  w: number,
  h: number,
  body: [number, number, number, number],
  radius: number,
): Shot => ({
  src,
  alt,
  w: w / 1.6,
  h: h / 1.6,
  body: body.map((v) => v / 1.6) as Shot["body"],
  radius,
});

export const SHOTS = {
  collapsed: at160(
    "/media/notch-collapsed.webp",
    "Oeca closed: track artwork, the time and a visualizer",
    438,
    63,
    [5, 0, 433, 63],
    14,
  ),
  collapsedTimer: at160(
    "/media/notch-collapsed-timer.webp",
    "Oeca closed, with a running countdown ring beside it",
    491,
    63,
    [5, 0, 490, 63],
    14,
  ),
  home: at160(
    "/media/space-home.webp",
    "The Home space: the current track with playback controls, a five-minute timer, and the time and date",
    1158,
    267,
    [7, 0, 1151, 239],
    24,
  ),
  homeTimer: at160(
    "/media/space-home-timer.webp",
    "The Home space with a five-minute countdown running",
    1158,
    267,
    [7, 0, 1151, 239],
    24,
  ),
  music: at160(
    "/media/space-music.webp",
    "The Music space: artwork, title, artist, visualizer and playback controls",
    726,
    307,
    [7, 0, 719, 279],
    24,
  ),
  settings: at160(
    "/media/notch-settings.webp",
    "Settings inside the notch: Always on top, Reduced motion and Open Full Settings",
    838,
    399,
    [7, 0, 831, 371],
    24,
  ),
  // The README's clipboard capture, taken at 100% scale.
  clipboard: {
    src: "/media/space-clipboard.webp",
    alt: "The Clipboard space with an image and three text items",
    w: 570,
    h: 266,
    body: [28, 0, 542, 238],
    radius: 24,
  } satisfies Shot,
};
