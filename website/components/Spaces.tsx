"use client";

import { useRef, useState, type KeyboardEvent } from "react";
import { ClipboardText, GearSix, House, MusicNotes, type Icon } from "@phosphor-icons/react";
import { NotchStage } from "./NotchStage";
import { SectionHead } from "./SectionHead";
import { CAPTURED_VERSION, SHOTS, type Shot } from "@/lib/site";
import { clipFull, clipToward, stageWidth } from "@/lib/notch";
import { MOTION_OK, SHELL_DURATION, SHELL_EASE, gsap, useGSAP } from "@/lib/gsap";

type Space = { id: string; label: string; icon: Icon; shot: Shot; title: string; body: string };

// Copy follows the source: timer stops (timer.rs), clipboard capacities
// (config.rs), session-scoped history (clipboard_store.rs), quick settings.
const SPACES: Space[] = [
  {
    id: "home",
    label: "Home",
    icon: House,
    shot: SHOTS.homeTimer,
    title: "Everything at once",
    body: "What’s playing with its controls, a countdown, and the time and date. Set the timer by dragging along its scale, from five seconds to twelve hours, and pause it whenever you like.",
  },
  {
    id: "music",
    label: "Music",
    icon: MusicNotes,
    shot: SHOTS.music,
    title: "Whatever is playing",
    body: "Artwork, title and artist from any app that uses the Windows media controls, with play, pause, previous, next and seek. The visualizer takes its colour from the artwork.",
  },
  {
    id: "clipboard",
    label: "Clipboard",
    icon: ClipboardText,
    shot: SHOTS.clipboard,
    title: "Your recent copies, kept",
    body: "Copied text and images, 5, 10 or 20 at a time. Click one to copy it again, or drag image files onto the notch to add them. History survives a restart and clears when you sign out.",
  },
  {
    id: "settings",
    label: "Settings",
    icon: GearSix,
    shot: SHOTS.settings,
    title: "Settings, right there",
    body: "Always on top and Reduced motion are a click away inside the notch. The full Settings window holds the rest.",
  },
];

const SHOT_LIST = SPACES.map((s) => s.shot);
const W = stageWidth(SHOT_LIST);

export function Spaces() {
  const root = useRef<HTMLElement>(null);
  const [active, setActive] = useState(0);
  const shown = useRef(0);

  // Hidden captures wait clipped to the visible one's notch, ready to grow.
  const { contextSafe } = useGSAP(
    () => {
      gsap.utils.toArray<HTMLImageElement>("[data-stage=spaces] img").forEach((img, i) => {
        gsap.set(img, { "--clip": i === 0 ? clipFull() : clipToward(SHOT_LIST[i], SHOT_LIST[0], W) });
      });
    },
    { scope: root },
  );

  // The selected-tab surface slides between tabs instead of jumping. It is
  // measured from the tab, so it also follows layout changes on resize.
  const placePill = useRef<(i: number, animate: boolean) => void>(() => {});
  useGSAP(
    () => {
      const list = root.current!.querySelector<HTMLElement>("[role=tablist]")!;
      const pill = list.querySelector<HTMLElement>("[data-tab-pill]")!;
      placePill.current = (i, animate) => {
        const tab = list.querySelector<HTMLElement>(`#tab-${SPACES[i].id}`)!;
        const to = { x: tab.offsetLeft, width: tab.offsetWidth };
        if (animate) gsap.to(pill, { ...to, duration: 0.5, ease: "power3.out", overwrite: true });
        else gsap.set(pill, { ...to, overwrite: true });
      };
      const ro = new ResizeObserver(() => placePill.current(shown.current, false));
      ro.observe(list);
      return () => ro.disconnect();
    },
    { scope: root },
  );

  /** Grow or shrink from the capture on screen into the chosen one. Every
   *  tween starts from the live value, so rapid clicks redirect smoothly. */
  // contextSafe is applied at click time, so GSAP work stays in the component's context
  const go = (next: number) =>
    contextSafe(() => {
      if (next === shown.current) return;
      shown.current = next;
      setActive(next);
      placePill.current(next, matchMedia(MOTION_OK).matches);
      const imgs = gsap.utils.toArray<HTMLImageElement>("[data-stage=spaces] img", root.current);
      const target = SHOT_LIST[next];
      const animate = matchMedia(MOTION_OK).matches;
      const d = animate ? SHELL_DURATION : 0;

      gsap.killTweensOf(imgs);
      imgs.forEach((img, i) => {
        if (i === next) {
          gsap.set(img, { zIndex: 2 });
          gsap.to(img, { opacity: 1, duration: d * 0.2, ease: "none" });
          gsap.to(img, { "--clip": clipFull(), duration: d, ease: SHELL_EASE });
        } else {
          gsap.set(img, { zIndex: 1 });
          gsap.to(img, { "--clip": clipToward(SHOT_LIST[i], target, W), duration: d, ease: SHELL_EASE });
          gsap.to(img, { opacity: 0, duration: d * 0.3, delay: d * 0.55, ease: "none" });
        }
      });

      if (animate) {
        gsap.fromTo(
          "[data-space-copy]",
          { autoAlpha: 0, y: 8 },
          { autoAlpha: 1, y: 0, duration: 0.5, overwrite: true },
        );
      }
    })();

  // Tabs pattern: arrow keys move between spaces, Home/End jump to the ends.
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const keys: Record<string, number> = {
      ArrowRight: (active + 1) % SPACES.length,
      ArrowLeft: (active - 1 + SPACES.length) % SPACES.length,
      Home: 0,
      End: SPACES.length - 1,
    };
    if (!(e.key in keys)) return;
    e.preventDefault();
    const next = keys[e.key];
    go(next);
    root.current?.querySelector<HTMLButtonElement>(`#tab-${SPACES[next].id}`)?.focus();
  };

  const space = SPACES[active];

  return (
    <section id="spaces" ref={root} className="section" aria-labelledby="spaces-title">
      <div className="shell">
        <SectionHead id="spaces-title" label="Spaces" title="Three spaces, one click apart">
          Open the notch and move between Home, Music and Clipboard, with settings a click away. Every view
          below is captured from the app.
        </SectionHead>

        <div data-reveal className="mt-14 flex flex-wrap items-center justify-between gap-4">
          <div
            role="tablist"
            aria-label="Oeca spaces"
            onKeyDown={onKey}
            className="relative flex w-fit max-w-full gap-0.5 rounded-xl p-1 shadow-[inset_0_0_0_1px_var(--color-line-strong)] sm:gap-1"
          >
            <span
              aria-hidden
              data-tab-pill
              className="absolute top-1 bottom-1 left-0 rounded-lg bg-surface shadow-[inset_0_0_0_1px_var(--color-line)]"
            />
            {SPACES.map((s, i) => {
              const selected = i === active;
              return (
                <button
                  key={s.id}
                  id={`tab-${s.id}`}
                  type="button"
                  role="tab"
                  aria-selected={selected}
                  aria-controls="space-panel"
                  tabIndex={selected ? 0 : -1}
                  onClick={() => go(i)}
                  className={`relative flex h-9 items-center gap-1.5 rounded-lg px-3 text-[0.875rem] font-medium whitespace-nowrap transition-colors duration-200 active:scale-[0.97] sm:gap-2 sm:px-3.5 ${
                    selected ? "text-paper" : "text-muted hover:text-paper"
                  }`}
                >
                  <s.icon
                    size={16}
                    weight={selected ? "fill" : "regular"}
                    className={`hidden transition-colors sm:block ${selected ? "text-accent" : ""}`}
                    aria-hidden
                  />
                  {s.label}
                </button>
              );
            })}
          </div>
          <p className="t-caption hidden md:block">
            Captured from Oeca {CAPTURED_VERSION}. The track and clipboard items are examples.
          </p>
        </div>

        <div id="space-panel" role="tabpanel" aria-labelledby={`tab-${space.id}`} className="mt-6">
          {/* A slice of desktop: every capture hangs from its top edge */}
          <div
            data-reveal
            className="overflow-hidden rounded-2xl bg-raised px-1 pb-10 shadow-[inset_0_0_0_1px_var(--color-line)]"
          >
            <NotchStage name="spaces" shots={SHOT_LIST} visible={0} current={active} sizes="724px" />
          </div>

          <div data-space-copy className="mt-8 grid gap-3 md:grid-cols-12 md:gap-10">
            <h3 className="t-heading md:col-span-6">{space.title}</h3>
            <p className="t-body max-w-xl md:col-span-6">{space.body}</p>
          </div>
          <p className="t-caption mt-6 md:hidden">
            Captured from Oeca {CAPTURED_VERSION}. The track and clipboard items are examples.
          </p>
        </div>
      </div>
    </section>
  );
}
