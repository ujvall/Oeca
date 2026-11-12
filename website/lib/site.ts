import { LATEST } from "./releases";

// Facts shown on the page. Each one is checked against the repository:
// version and installer = the newest entry in releases.ts (GitHub Releases);
// requirements = installer/Oeca.iss (MinVersion=10.0, x64compatible,
// PrivilegesRequired=lowest). A new release is added in releases.ts only.
export const VERSION = LATEST.version;

export const REPO_URL = "https://github.com/ujvall/Oeca";
export const RELEASES_URL = `${REPO_URL}/releases`;
export const DOWNLOAD_URL = LATEST.installerUrl ?? `${REPO_URL}/releases`;
export const LICENSE_URL = `${REPO_URL}/blob/master/LICENSE`;
export const REQUIREMENTS = "Windows 10 or later, 64-bit";

// Absolute base for Open Graph URLs: Vercel sets the production domain at build time.
export const SITE_URL = process.env.VERCEL_PROJECT_PRODUCTION_URL
  ? `https://${process.env.VERCEL_PROJECT_PRODUCTION_URL}`
  : "http://localhost:3000";
