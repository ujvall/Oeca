// Modified for Oeca Nearby from LocalSend Web (Apache-2.0): see OECA.md.
// https://nuxt.com/docs/api/configuration/nuxt-config
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { headersFile, inlineScriptHashes, vercelConfig, withPolicy } from "./csp";
import { endpoints } from "./endpoints";
import * as built from "./notices";
const { notices } = built;

// oeca: General Sans, Oeca's typeface (Indian Type Foundry, ITF Free Font
// License), served like the native app embeds it: copied at build time from
// ../assets/fonts, where it is git-ignored because the license does not allow
// publishing the files in this repository. Without it the site uses Hubot Sans.
const GENERAL_SANS = ["Regular", "Medium", "Semibold"] as const;
const here = fileURLToPath(new URL(".", import.meta.url));
const fontCache = join(here, "node_modules", ".cache", "oeca-general-sans");
const generalSans = GENERAL_SANS.every((w) => existsSync(join(here, "..", "assets", "fonts", `GeneralSans-${w}.otf`)));
if (generalSans) {
  mkdirSync(fontCache, { recursive: true });
  for (const w of GENERAL_SANS) {
    copyFileSync(join(here, "..", "assets", "fonts", `GeneralSans-${w}.otf`), join(fontCache, `general-sans-${w.toLowerCase()}.otf`));
  }
}
const GENERAL_SANS_NOTICE = generalSans
  ? `
Typeface: General Sans by the Indian Type Foundry, used under the ITF Free
Font License (https://www.fontshare.com/licenses/itf-ffl), served as
/fonts/general-sans-*.otf.
`
  : "";

const NOTICES_HEADER = `Oeca Hop (web): third-party notices
==============================================================================

This site is Oeca Hop, adapted from LocalSend Web
(https://github.com/localsend/web, Copyright (c) LocalSend contributors,
Apache License 2.0, reproduced at the end), with Oeca's changes (MIT License;
see OECA.md in the Oeca source).

Typeface: Hubot Sans, Copyright 2023 The Hubot Sans Project Authors
(https://github.com/github/hubot-sans), SIL Open Font License 1.1, bundled as
/fonts/hubot-sans-*.woff2 with its license at /fonts/OFL.txt.
${GENERAL_SANS_NOTICE}
The packages below are the ones whose code is in this site's files, each with
its own license and notice files. Generated at build time (notices.ts).
`;

// oeca: public servers only in a production build, unless set (endpoints.ts)
const { signalingUrl, stunServers } = endpoints(process.env);

export default defineNuxtConfig({
  compatibilityDate: "2024-11-01",
  modules: [
    "@nuxtjs/tailwindcss",
    "@nuxtjs/i18n",
  ],
  // oeca: Oeca Nearby's look (the module's default path is outside app/)
  tailwindcss: { cssPath: "~/assets/css/tailwind.css" },
  // oeca: no DevTools (its RPC server is a remote-code risk while a dev server runs)
  devtools: { enabled: false },
  // oeca: no inline <style>: the Content-Security-Policy allows only this origin's files
  features: { inlineStyles: false },
  // oeca: no debug logging in a build, whatever a future change adds (warnings and errors stay)
  vite: {
    // oeca: no chatty logs in the bundle (warnings and errors stay). Vite 8
    // builds with Rolldown, which ignores esbuild's `pure`
    build: {
      rolldownOptions: {
        treeshake: { manualPureFunctions: ["console.log", "console.info", "console.debug", "console.trace"] },
      },
    },
    plugins: [
      notices({
        // The fork's LICENSE (Apache-2.0) after the header, without carriage returns
        header: [NOTICES_HEADER, readFileSync(join(__dirname, "LICENSE"), "utf8").split(String.fromCharCode(13)).join("")].join(String.fromCharCode(10)),
        // Tailwind's generated CSS ships, though it is no JS module
        always: [join(__dirname, "node_modules", "tailwindcss")],
      }),
    ],
  },
  runtimeConfig: {
    public: {
      signalingUrl,
      stunServers,
    },
  },
  app: {
    head: {
      // oeca: edge to edge on phones (the page pads itself by the safe areas); the
      // keyboard resizes the layout; the status bar matches the page (app.vue)
      viewport: "width=device-width, initial-scale=1, viewport-fit=cover, interactive-widget=resizes-content",
      meta: [
        { name: "theme-color", content: "#edeef1", media: "(prefers-color-scheme: light)" },
        { name: "theme-color", content: "#000000", media: "(prefers-color-scheme: dark)" },
        {
          name: "apple-mobile-web-app-title",
          content: "Oeca Hop",
        },
      ],
      link: [
        {
          rel: "icon",
          href: "/favicon.ico",
        },
        {
          rel: "apple-touch-icon",
          sizes: "180x180",
          href: "/apple-touch-icon.png",
        },
        // oeca: installable from a static manifest; no service worker (OECA.md)
        {
          rel: "manifest",
          href: "/manifest.webmanifest",
        },
      ],
    },
  },
  i18n: {
    // oeca: Nearby's own origin, apart from the oeca.app website (OECA.md, Origin)
    baseUrl: "https://nearby.oeca.app",
    strategy: "prefix_except_default",
    defaultLocale: "en",
    locales: [
      {
        code: "de",
        language: "de-DE",
        file: "de.json",
        name: "Deutsch",
      },
      {
        code: "en",
        language: "en-US",
        file: "en.json",
        name: "English",
        isCatchallLocale: true,
      },
      {
        code: "es",
        language: "es-ES",
        file: "es.json",
        name: "Español",
      },
      {
        code: "fa",
        language: "fa-IR",
        file: "fa.json",
        name: "فارسی",
      },
      {
        code: "hu",
        language: "hu-HU",
        file: "hu.json",
        name: "Magyar",
      },
      {
        code: "it",
        language: "it-IT",
        file: "it.json",
        name: "Italiano",
      },
      {
        code: "km",
        language: "km-KH",
        file: "km.json",
        name: "ភាសាខ្មែរ",
      },
      {
        code: "ko",
        language: "ko-KR",
        file: "ko.json",
        name: "한국어",
      },
      {
        code: "no",
        language: "no-NO",
        file: "no.json",
        name: "Norsk",
      },
      {
        code: "pt",
        language: "pt-BR",
        file: "pt.json",
        name: "Português",
      },
      {
        code: "sk",
        language: "sk-SK",
        file: "sk.json",
        name: "Slovenčina",
      },
      {
        code: "tr",
        language: "tr-TR",
        file: "tr.json",
        name: "Türkçe",
      },
      {
        code: "zh-CN",
        language: "zh-CN",
        file: "zh-CN.json",
        name: "简体中文",
      },
      {
        code: "ja-JP",
        language: "ja-JP",
        file: "ja-JP.json",
        name: "日本語"
      }
    ],
  },
  nitro: {
    publicAssets: generalSans ? [{ dir: fontCache, baseURL: "/fonts", maxAge: 0 }] : [],
    prerender: {
      routes: ["/"],
      autoSubfolderIndex: false,
    },
  },
  // oeca: every page carries its Content-Security-Policy; `_headers` and
  // `vercel.json` repeat it (with frame-ancestors) for the host (csp.ts, OECA.md)
  hooks: {
    "nitro:init"(nitro) {
      const hashes = new Set<string>();
      nitro.hooks.hook("prerender:generate", (route) => {
        if (typeof route.contents !== "string" || !route.fileName?.endsWith(".html")) return;
        for (const h of inlineScriptHashes(route.contents)) hashes.add(h);
        route.contents = withPolicy(route.contents, signalingUrl);
      });
      nitro.hooks.hook("prerender:done", () => {
        writeFileSync(join(nitro.options.output.publicDir, "_headers"), headersFile([...hashes].sort(), signalingUrl));
        writeFileSync(join(nitro.options.output.publicDir, "vercel.json"), vercelConfig([...hashes].sort(), signalingUrl));
        // The third-party notices of what this site's files contain (notices.ts)
        if (!built.generated) throw new Error("No third-party notices were generated: the site is not built without them");
        writeFileSync(join(nitro.options.output.publicDir, "THIRD-PARTY-NOTICES.txt"), built.generated);
      });
    },
  },
});
