import type { Config } from "tailwindcss";

// oeca: colors are the tokens in app/assets/css/tailwind.css
const token = (name: string) => `rgb(var(--${name}) / <alpha-value>)`;

export default <Partial<Config>>{
  darkMode: "class",
  // oeca: `hover:` only where the pointer can hover, so a tap never leaves one stuck
  future: { hoverOnlyWhenSupported: true },
  theme: {
    extend: {
      colors: Object.fromEntries(
        [
          "bg",
          "object",
          "object-2",
          "sheet",
          "group",
          "sunk",
          "ink",
          "ink-2",
          "ink-3",
          "line",
          "cta",
          "on-cta",
          "notch",
          "signal",
          "warn",
          "warn-bg",
          "bad",
          "bad-bg",
          "on-bad",
          "good",
          "good-bg",
        ].map((n) => [n, token(n)]),
      ),
    },
  },
};
