// Published Oeca releases, newest first. Copied from GitHub Releases
// (https://github.com/ujvall/Oeca/releases), the only source for this file:
// no drafts, pre-releases, unreleased commits or Cargo.toml guesses.
//
// To add a release: put a new entry at the top with its tag, the publish date
// from GitHub (UTC), the release page URL, and the installer URL only if the
// release really has that asset. Keep the published notes' wording; sort them
// into Added / Improved / Fixed / Changed. Leave `sections` empty when no
// notes were published (the page then says so).

export type SectionTitle = "Added" | "Improved" | "Fixed" | "Changed";

export type Release = {
  version: string;
  tag: string;
  /** GitHub's published_at, ISO 8601 UTC. */
  publishedAt: string;
  url: string;
  /** Verified installer asset; omit when the release has none. */
  installerUrl?: string;
  summary?: string;
  sections: { title: SectionTitle; items: string[] }[];
  /** Other published remarks (requirements, signing), kept as written. */
  notes?: string[];
};

export const RELEASES: Release[] = [
  {
    version: "0.3.0",
    tag: "v0.3.0",
    publishedAt: "2026-10-17T11:16:59Z",
    url: "https://github.com/ujvall/Oeca/releases/tag/v0.3.0",
    installerUrl: "https://github.com/ujvall/Oeca/releases/download/v0.3.0/Oeca-Setup-0.3.0.exe",
    summary: "A volume display in the notch, Focus and Break timers, Energy Saver awareness, and optional update checks.",
    sections: [
      {
        title: "Added",
        items: [
          "A volume display in the notch: the volume keys show the level and mute state in place of the Windows volume flyout",
          "Energy Saver awareness: while it is on, Oeca moves as if Reduced motion were on, and the notch briefly shows when it turns on or off",
          "Focus and Break modes for the timer, each with its own duration from 30 seconds to 2 hours, remembered between launches",
          "Update notices: when a check finds a newer Oeca, a notice appears under the notch, then a download icon in place of the artwork; either opens the release page",
          "Check for updates in Settings, under About: the installed version, the result of the last check, a link to a newer release, and an optional daily check",
        ],
      },
      {
        title: "Improved",
        items: [
          "The timer's numerals and labels use the rounded Sunghyun Sans typeface",
          "Changing a running timer's duration glides to the new time instead of jumping",
        ],
      },
      {
        title: "Changed",
        items: [
          "Oeca can now contact GitHub, only to check for updates: when you choose Check for updates, or once a day if you turn on Check for updates automatically (off by default). Each check is one HTTPS request with no account, telemetry or personal data. Nothing is downloaded or installed for you",
        ],
      },
    ],
    notes: [
      "Requires Windows 10 or Windows 11, with a compatible 64-bit system.",
      "Windows may display a security warning because the installer is not code-signed.",
    ],
  },
  {
    version: "0.2.0",
    tag: "v0.2.0",
    publishedAt: "2026-10-15T17:00:00Z",
    url: "https://github.com/ujvall/Oeca/releases/tag/v0.2.0",
    installerUrl: "https://github.com/ujvall/Oeca/releases/download/v0.2.0/Oeca-Setup.exe",
    summary:
      "Drag images onto the notch, close it instantly, and smoother animation on high-refresh and high-resolution displays.",
    sections: [
      {
        title: "Added",
        items: [
          "Drag images from File Explorer onto the notch to add them to your clipboard history and copy them to the clipboard, with an \u201cItem copied to clipboard\u201d confirmation",
          "Clear messages when a dropped file is not an image, cannot be read or is too large",
          "A 0 second closing delay: the notch closes as soon as you click elsewhere (the new default)",
          "New icons, drawn for Oeca",
        ],
      },
      {
        title: "Improved",
        items: [
          "Smoother animation on 120 Hz and 144 Hz displays: frames now follow the display\u2019s refresh rate",
          "Faster rendering at high display scaling, such as on 4K screens",
          "Text uses General Sans on every PC, without installing the font",
        ],
      },
      {
        title: "Fixed",
        items: [
          "Dropping a file onto the notch no longer briefly freezes File Explorer",
          "Dragging a file onto the open notch no longer counts as a click outside it",
        ],
      },
      {
        title: "Changed",
        items: [
          "Oeca is free and open source under the MIT License",
          "Your existing closing delay setting is kept when you update",
        ],
      },
    ],
    notes: [
      "Requires Windows 10 or Windows 11, with a compatible 64-bit system.",
      "Windows may display a security warning because the installer is not code-signed.",
    ],
  },
  {
    version: "0.1.0",
    tag: "v0.1.0",
    publishedAt: "2026-10-13T16:01:00Z",
    url: "https://github.com/ujvall/Oeca/releases/tag/v0.1.0",
    installerUrl: "https://github.com/ujvall/Oeca/releases/download/v0.1.0/Oeca-Setup.exe",
    summary: "A lightweight, native Windows notch application built with Rust.",
    // The first release: its "Features" list is everything it added.
    sections: [
      {
        title: "Added",
        items: [
          "Native Windows UI with smooth animations",
          "Music controls and media visualizer",
          "Clipboard history",
          "Countdown timer",
          "Customizable appearance and settings",
          "Optional launch on Windows startup",
        ],
      },
    ],
    notes: [
      "Requires Windows 10 or Windows 11, with a compatible 64-bit system.",
      "Windows may display a security warning because the installer is not code-signed.",
    ],
  },
];

export const LATEST = RELEASES[0];

/** The next version, prepared in the repository but not published on GitHub
 *  Releases: the changelog shows it as unreleased, never as the latest
 *  version or a download. When it is published, move it into RELEASES (with
 *  its tag, date, release page and verified installer) and set this to
 *  undefined. */
export type Upcoming = Pick<Release, "version" | "summary" | "sections" | "notes">;

export const UPCOMING: Upcoming | undefined = undefined;

/** "v0.1.0" -> "v0-1-0", a stable in-page anchor. */
export const releaseId = (r: Release) => r.tag.replace(/\./g, "-");

export const formatDate = (iso: string) =>
  new Intl.DateTimeFormat("en-US", { dateStyle: "long", timeZone: "UTC" }).format(new Date(iso));
