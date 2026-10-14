"use client";

import { useEffect, useRef, useState } from "react";
import { ArrowUpRight, CaretRight, DownloadSimple } from "@phosphor-icons/react";
import { formatDate, releaseId, type Release } from "@/lib/releases";
import { MOTION_OK, ScrollTrigger, gsap, useGSAP } from "@/lib/gsap";

// Pill and panel widths: CLOSED fits "0.10.10 Latest" and the arrow.
const CLOSED_W = "8.75rem";
const OPEN_W = "13rem";
const RAIL = "(min-width: 64rem)";

export function ReleaseTimeline({ releases }: { releases: Release[] }) {
  const root = useRef<HTMLDivElement>(null);
  const [active, setActive] = useState(0);
  const [open, setOpen] = useState(false);

  // Marks the release being read. A ScrollTrigger per entry flips the index as
  // it crosses the upper part of the viewport (no scroll listener).
  const { contextSafe } = useGSAP(
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

  /** Opens to the right, closes to the left: the rail widens (its arrow rides
   *  the edge and turns to point back) and the list fades in. Short and sharp:
   *  expo.out lands most of the motion in the first frames, no overshoot.
   *  Tweens start from the values on screen, so rapid clicks reverse cleanly. */
  const toggle = (next: boolean) =>
    contextSafe(() => {
      setOpen(next);
      const d = !matchMedia(MOTION_OK).matches ? 0 : next ? 0.34 : 0.26;
      const ease = "expo.out";
      gsap.to("[data-box]", { width: next ? OPEN_W : CLOSED_W, duration: d, ease });
      gsap.to("[data-arrow]", { rotation: next ? 180 : 0, duration: d, ease });
      // Wide screens: the rail is already full height. Small screens: the
      // list unfolds in the flow below the header.
      if (!matchMedia(RAIL).matches) gsap.to("[data-list]", { height: next ? "auto" : 0, duration: d, ease });
      gsap.to("[data-list] > ol", {
        autoAlpha: next ? 1 : 0,
        duration: next ? 0.2 : 0.12,
        delay: next && d ? 0.08 : 0,
        ease: "power2.out",
      });
    })();

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && toggle(false);
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  });

  const current = releases[active];

  return (
    <div ref={root} className="mt-12 md:mt-16">
      {/* Wide screens: a full-height rail on the left edge of <main>, its top
          level with the "Changelog" heading (7.5rem = nav + page padding), in a
          margin the page keeps free (.with-rail), so it never covers the text,
          and it stops above the footer. Small screens: in the flow above the
          releases. */}
      <nav aria-label="Releases" className="z-30 mb-10 lg:absolute lg:top-0 lg:bottom-6 lg:left-6 lg:mb-0">
        <div
          data-box
          style={{ width: CLOSED_W }}
          className="flex flex-col overflow-hidden rounded-2xl bg-[#151515] shadow-[inset_0_0_0_1px_var(--color-line-strong)] lg:sticky lg:top-[7.5rem] lg:h-[min(calc(100dvh-9rem),calc(100%-7.5rem))]"
        >
          <button
            type="button"
            aria-expanded={open}
            aria-controls="release-list"
            onClick={() => toggle(!open)}
            className="flex h-11 w-full shrink-0 items-center justify-between gap-3 rounded-2xl pr-3 pl-4 text-[0.9375rem] focus-visible:outline-offset-[-2px]"
          >
            <span className="whitespace-nowrap tracking-[-0.01em]">
              {open ? (
                "Timeline"
              ) : (
                <span className="tabular-nums">
                  <span className="sr-only">Timeline, reading </span>
                  {current.version}
                  {active === 0 && <sup className="ml-1 text-[0.625rem] text-muted">Latest</sup>}
                </span>
              )}
            </span>
            <span data-arrow aria-hidden className="grid size-6 place-items-center text-muted">
              <CaretRight size={14} weight="bold" />
            </span>
          </button>

          <div
            data-list
            id="release-list"
            inert={!open}
            className="h-0 overflow-hidden lg:h-auto lg:min-h-0 lg:flex-1 lg:overflow-y-auto"
          >
            <ol className="invisible grid gap-1 px-2 pb-2 opacity-0" style={{ width: OPEN_W }}>
              {releases.map((r, i) => {
                const on = i === active;
                return (
                  <li key={r.tag}>
                    <a
                      href={`#${releaseId(r)}`}
                      aria-current={on ? "location" : undefined}
                      onClick={() => toggle(false)}
                      className="group flex h-9 items-center gap-3 rounded-xl px-2 whitespace-nowrap text-[0.9375rem] transition-colors hover:bg-white/[0.04]"
                    >
                      <span
                        aria-hidden
                        className={`block h-px transition-[width,background-color] duration-300 ease-out motion-reduce:transition-none ${
                          on ? "w-6 bg-paper" : "w-3 bg-paper/30 group-hover:bg-paper"
                        }`}
                      />
                      <span
                        className={`tabular-nums ${on ? "text-paper" : "text-muted group-hover:text-paper"}`}
                      >
                        {r.version}
                        {i === 0 && <sup className="ml-1 text-[0.625rem] text-muted">Latest</sup>}
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
      <div className="flex items-center gap-3 text-[0.875rem] text-muted">
        <span aria-hidden className="block h-px w-8 bg-paper/40" />
        <time dateTime={r.publishedAt}>{formatDate(r.publishedAt)}</time>
        {latest && <span className="text-paper">Latest</span>}
      </div>

      <h2
        id={`${releaseId(r)}-title`}
        className="mt-5 text-[clamp(2.5rem,1.9rem+2.4vw,3.75rem)] leading-none font-semibold tracking-[-0.035em] tabular-nums"
      >
        <span className="sr-only">Oeca </span>
        {r.version}
      </h2>
      {r.summary && <p className="t-lead mt-5 max-w-xl">{r.summary}</p>}

      <div className="mt-10 grid gap-8">
        {r.sections.length === 0 && (
          <p className="t-body">No release notes were published for this version.</p>
        )}
        {r.sections.map((s) => (
          <section key={s.title} className="grid gap-3 md:grid-cols-[8rem_1fr] md:gap-6">
            <h3 className="text-[0.9375rem] font-medium">{s.title}</h3>
            <ul className="grid gap-2.5">
              {s.items.map((item) => (
                <li key={item} className="flex gap-3 text-paper/85">
                  <span aria-hidden className="mt-[0.8em] block h-px w-3 shrink-0 bg-paper/30" />
                  {item}
                </li>
              ))}
            </ul>
          </section>
        ))}
        {r.notes && r.notes.length > 0 && (
          <section className="grid gap-3 md:grid-cols-[8rem_1fr] md:gap-6">
            <h3 className="text-[0.9375rem] font-medium text-muted">Notes</h3>
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

      <div className="mt-10 flex flex-col gap-3 sm:flex-row md:ml-[calc(8rem+1.5rem)]">
        {r.installerUrl && (
          <a className="btn btn-primary btn-sm" href={r.installerUrl}>
            <DownloadSimple size={16} weight="bold" aria-hidden />
            Download {r.version}
          </a>
        )}
        <a className="btn btn-ghost btn-sm" href={r.url}>
          Release on GitHub
          <ArrowUpRight size={14} weight="bold" aria-hidden />
        </a>
      </div>
    </article>
  );
}
