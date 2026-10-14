"use client";

import { useRef, useState, type KeyboardEvent } from "react";
import { ClipboardText, GearSix, House, MusicNotes, type Icon } from "@phosphor-icons/react";
import { NotchStage } from "./NotchStage";
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
    title: "Everything at once.",
    body: "What’s playing with its controls, a countdown, and the time and date. Set the timer by dragging along its scale, from five seconds to twelve hours, and pause it whenever you like.",
  },
  {
    id: "music",
    label: "Music",
    icon: MusicNotes,
    shot: SHOTS.music,
    title: "Whatever is playing.",
    body: "Artwork, title and artist from any app that uses the Windows media controls, with play, pause, previous, next and seek. The visualizer takes its colour from the artwork.",
  },
  {
    id: "clipboard",
    label: "Clipboard",
    icon: ClipboardText,
    shot: SHOTS.clipboard,
    title: "Your recent copies, kept.",
    body: "Copied text and images, 5, 10 or 20 at a time. Click one to copy it again, or drag image files onto the notch to add them. History survives a restart and clears when you sign out.",
  },
  {
    id: "settings",
    label: "Settings",
    icon: GearSix,
    shot: SHOTS.settings,
    title: "Settings, right there.",
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

  /** Grow or shrink from the capture on screen into the chosen one. Every
   *  tween starts from the live value, so rapid clicks redirect smoothly. */
  // contextSafe is applied at click time, so GSAP work stays in the component's context
  const go = (next: number) =>
    contextSafe(() => {
      if (next === shown.current) return;
      shown.current = next;
      setActive(next);
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
    <section id="spaces" ref={root} className="pt-28 md:pt-40" aria-labelledby="spaces-title">
      <div className="shell">
        <div className="mx-auto max-w-2xl text-center">
          <h2 id="spaces-title" className="t-title">
            Three spaces, one click apart.
          </h2>
          <p className="t-lead mt-5">
            Open the notch and move between Home, Music and Clipboard, with settings a click away. Every view
            below is captured from the app.
          </p>
        </div>

        <div
          role="tablist"
          aria-label="Oeca spaces"
          onKeyDown={onKey}
          className="mx-auto mt-12 flex w-fit max-w-full gap-0.5 rounded-full p-1 shadow-[inset_0_0_0_1px_var(--color-line)] sm:gap-1"
        >
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
                className={`flex h-10 items-center gap-1.5 rounded-full px-3.5 text-[0.875rem] font-medium whitespace-nowrap transition-colors duration-200 sm:gap-2 sm:px-4 sm:text-[0.9063rem] ${
                  selected ? "bg-white/[0.09] text-paper" : "text-muted hover:text-paper"
                }`}
              >
                <s.icon
                  size={18}
                  weight={selected ? "fill" : "regular"}
                  className="hidden sm:block"
                  aria-hidden
                />
                {s.label}
              </button>
            );
          })}
        </div>

        <div id="space-panel" role="tabpanel" aria-labelledby={`tab-${space.id}`} className="mt-10">
          {/* The screen's top edge: every capture hangs from it, as on the desktop */}
          <div className="border-t border-line-strong px-1">
            <NotchStage name="spaces" shots={SHOT_LIST} visible={0} current={active} sizes="724px" />
          </div>

          <div data-space-copy className="mx-auto mt-8 max-w-xl text-center">
            <h3 className="t-heading">{space.title}</h3>
            <p className="t-body mt-3">{space.body}</p>
          </div>
          <p className="t-caption mt-8 text-center">
            Captured from Oeca {CAPTURED_VERSION}. The track and clipboard items are examples.
          </p>
        </div>
      </div>
    </section>
  );
}
