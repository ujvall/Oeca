import type { Metadata } from "next";
import { Nav } from "@/components/Nav";
import { Footer } from "@/components/Footer";
import { ReleaseTimeline, UpcomingEntry } from "@/components/ReleaseTimeline";
import { RELEASES, UPCOMING } from "@/lib/releases";
import { RELEASES_URL } from "@/lib/site";

const title = "Changelog: Oeca";
const description = "Every official Oeca release for Windows, newest first, with its notes and installer.";

export const metadata: Metadata = {
  title,
  description,
  alternates: { canonical: "/changelog/" },
  // Overriding openGraph drops the inherited image, so it is named again here
  openGraph: {
    type: "website",
    siteName: "Oeca",
    title,
    description,
    url: "/changelog/",
    images: [
      { url: "/opengraph-image.png", width: 1200, height: 630, alt: "Oeca: a native notch for Windows" },
    ],
  },
  twitter: { card: "summary_large_image", title, description, images: ["/opengraph-image.png"] },
};

export default function Changelog() {
  return (
    <>
      <a
        href="#releases"
        className="sr-only z-50 rounded-full bg-paper px-4 py-2 text-ink focus:not-sr-only focus:fixed focus:top-3 focus:left-3"
      >
        Skip to releases
      </a>
      <Nav />
      <main id="releases" className="relative pt-16">
        <div className="shell with-rail pt-10 pb-28 md:pt-14 md:pb-40">
          <header className="max-w-2xl">
            <h1 className="t-title">Changelog</h1>
            <p className="t-lead mt-5">
              The official Oeca releases, newest first, as published on{" "}
              <a className="link" href={RELEASES_URL}>
                GitHub Releases
              </a>
              .
            </p>
          </header>
          {UPCOMING && <UpcomingEntry upcoming={UPCOMING} />}
          <ReleaseTimeline releases={RELEASES} />
        </div>
      </main>
      <Footer />
    </>
  );
}
