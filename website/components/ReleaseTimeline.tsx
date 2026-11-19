"use client";

import { useRef, useState } from "react";
import { formatDate, releaseId, type Release, type Upcoming } from "@/lib/releases";
import { ScrollTrigger, gsap, useGSAP } from "@/lib/gsap";

// The rail's width: fits "0.10.10 Latest" with its marker.
const RAIL_W = "13rem";

export function ReleaseTimeline({ releases }: { releases: Release[] }) {
  const root = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState(0);

  // Marks the release being read. A ScrollTrigger per entry flips the index as
  // it crosses the upper part of the viewport (no scroll listener).
  useGSAP(
    () => {
      gsap.utils.toArray<HTMLElement>("[data-release]").forEach((el, i) => {
        ScrollTrigger.create({
          trigger: el,
          start: "top 45%",
          end: "bottom 45%",
          onToggle: (self) => self.isActive && setActive(i),
        });
      });
    },
    { scope: root },
  );

  return (
    <div ref={root} className="mt-12 md:mt-16">
      {/* Always open. Wide screens: a full-height rail on the left edge of
          <main>, its top level with the "Changelog" heading (7.5rem = nav +
          page padding), in a margin the page keeps free (.with-rail), so it
          never covers the text, and it stops above the footer. Small screens:
          in the flow above the releases. */}
      <nav aria-labelledby="release-list-title" className="z-30 mb-10 lg:absolute lg:top-0 lg:bottom-6 lg:left-6 lg:mb-0">
        <div
          style={{ width: RAIL_W }}
          className="flex flex-col overflow-hidden rounded-2xl bg-raised shadow-[inset_0_0_0_1px_var(--color-line-strong)] lg:sticky lg:top-[7.5rem] lg:h-[min(calc(100dvh-9rem),calc(100%-7.5rem))]"
        >
          <h2
            id="release-list-title"
            className="t-mono flex h-11 shrink-0 items-center border-b border-line pl-4 font-normal text-muted"
          >
            Timeline
          </h2>

          <div data-lenis-prevent className="lg:min-h-0 lg:flex-1 lg:overflow-y-auto">
            <ol className="grid gap-0.5 p-2">
              {releases.map((r, i) => {
                const on = i === active;
                return (
                  <li key={r.tag}>
                    <a
                      href={`#${releaseId(r)}`}
                      aria-current={on ? "location" : undefined}
                      className={`group flex h-9 items-center gap-3 rounded-lg px-2 whitespace-nowrap transition-colors ${on ? "bg-surface" : "hover:bg-white/[0.04]"}`}
                    >
                      <span
                        aria-hidden
                        className={`block h-px transition-[width,background-color] duration-300 ease-out motion-reduce:transition-none ${
                          on ? "w-6 bg-accent" : "w-3 bg-paper/30 group-hover:bg-paper"
                        }`}
                      />
                      <span
                        className={`t-mono ${on ? "text-paper" : "text-muted group-hover:text-paper"}`}
                      >
                        <span className="sr-only">Oeca </span>
                        {r.version}
                        {i === 0 && <span className="ml-2 text-[0.6875rem] text-accent">Latest</span>}
                      </span>
                    </a>
                  </li>
                );
              })}
            </ol>
          </div>
        </div>
      </nav>

      <ol className="min-w-0 max-w-3xl">
        {releases.map((r, i) => (
          <li
            key={r.tag}
            id={releaseId(r)}
            data-release
            className="scroll-mt-28 border-t border-line pt-12 pb-4 first:border-t-0 first:pt-0 [&+li]:mt-16"
          >
            <ReleaseEntry release={r} latest={i === 0} />
          </li>
        ))}
      </ol>
    </div>
  );
}

function ReleaseEntry({ release: r, latest }: { release: Release; latest: boolean }) {
  return (
    <article aria-labelledby={`${releaseId(r)}-title`}>
      <div className="t-mono flex items-center gap-3 text-muted">
        <time dateTime={r.publishedAt}>{formatDate(r.publishedAt)}</time>
        {latest && (
          <span className="rounded-md px-1.5 py-0.5 text-paper shadow-[inset_0_0_0_1px_var(--color-line-strong)]">
            Latest
          </span>
        )}
      </div>

      <h2
        id={`${releaseId(r)}-title`}
        className="mt-5 text-[clamp(2.5rem,1.9rem+2.4vw,3.75rem)] leading-none font-semibold tracking-[-0.035em] tabular-nums"
      >
        <span className="sr-only">Oeca </span>
        {r.version}
      </h2>
      {r.summary && <p className="t-lead mt-5 max-w-xl">{r.summary}</p>}

      <ReleaseNotes notes={r} />

      <div className="mt-10 flex flex-col gap-3 sm:flex-row md:ml-[calc(8rem+1.5rem)]">
        {r.installerUrl && (
          <a className="btn btn-primary btn-sm" href={r.installerUrl}>
            Download {r.version}
          </a>
        )}
        <a className="btn btn-ghost btn-sm" href={r.url}>
          Release on GitHub
        </a>
      </div>
    </article>
  );
}

/** The next version, prepared but not published: its notes only (no date,
 *  download or release page), clearly marked unreleased. */
export function UpcomingEntry({ upcoming: u }: { upcoming: Upcoming }) {
  const id = `v${u.version.replace(/\./g, "-")}-unreleased`;
  return (
    <article aria-labelledby={`${id}-title`} className="mt-12 max-w-3xl border-b border-line pb-12 md:mt-16">
      <div className="t-mono flex items-center gap-3 text-muted">
        <span className="rounded-md px-1.5 py-0.5 text-paper shadow-[inset_0_0_0_1px_var(--color-line-strong)]">
          Unreleased
        </span>
      </div>
      <h2
        id={`${id}-title`}
        className="mt-5 text-[clamp(2.5rem,1.9rem+2.4vw,3.75rem)] leading-none font-semibold tracking-[-0.035em] tabular-nums"
      >
        <span className="sr-only">Oeca </span>
        {u.version}
      </h2>
      {u.summary && <p className="t-lead mt-5 max-w-xl">{u.summary}</p>}
      <ReleaseNotes notes={u} />
    </article>
  );
}

function ReleaseNotes({ notes: r }: { notes: Upcoming }) {
  return (
    <div className="mt-10 grid gap-8">
      {r.sections.length === 0 && (
        <p className="t-body">No release notes were published for this version.</p>
      )}
      {r.sections.map((s) => (
        <section key={s.title} className="grid gap-3 md:grid-cols-[8rem_1fr] md:gap-6">
          <h3 className="t-mono pt-0.5 text-muted">{s.title}</h3>
          <ul className="grid gap-2.5">
            {s.items.map((item) => (
              <li key={item} className="flex gap-3 text-paper/85">
                <span aria-hidden className="mt-[0.7em] block size-1 shrink-0 rounded-[1px] bg-paper/35" />
                {item}
              </li>
            ))}
          </ul>
        </section>
      ))}
      {r.notes && r.notes.length > 0 && (
        <section className="grid gap-3 md:grid-cols-[8rem_1fr] md:gap-6">
          <h3 className="t-mono pt-0.5 text-muted">Notes</h3>
          <ul className="grid gap-2.5">
            {r.notes.map((n) => (
              <li key={n} className="t-body text-[0.9688rem]">
                {n}
              </li>
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}
