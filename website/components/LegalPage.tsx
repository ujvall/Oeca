import { Nav } from "./Nav";
import { Footer } from "./Footer";

/** Shared frame for the policy pages: the site's nav and footer around a
 *  readable column of prose (styled by `.legal` in globals.css). */
export function LegalPage({
  title,
  lead,
  updated,
  children,
}: {
  title: string;
  lead: React.ReactNode;
  /** ISO date and its display form, e.g. ["2026-10-15", "15 October 2026"]. */
  updated: [string, string];
  children: React.ReactNode;
}) {
  return (
    <>
      <a
        href="#content"
        className="sr-only z-50 rounded-full bg-paper px-4 py-2 text-ink focus:not-sr-only focus:fixed focus:top-3 focus:left-3"
      >
        Skip to content
      </a>
      <Nav />
      <main id="content" className="pt-16">
        <div className="shell pt-10 pb-28 md:pt-14 md:pb-40">
          <header className="max-w-2xl">
            <h1 className="t-title">{title}</h1>
            <p className="t-lead mt-5">{lead}</p>
            <p className="t-caption mt-4">
              Last updated <time dateTime={updated[0]}>{updated[1]}</time>
            </p>
          </header>
          <article className="legal mt-12 max-w-2xl md:mt-16">{children}</article>
        </div>
      </main>
      <Footer />
    </>
  );
}
