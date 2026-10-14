"use client";

import { useRef, useState } from "react";
import { DownloadSimple } from "@phosphor-icons/react";
import { NotchStage } from "./NotchStage";
import { DOWNLOAD_URL, SHOTS } from "@/lib/site";
import { clipFull, clipToward, stageWidth } from "@/lib/notch";
import { MOTION_OK, SHELL_DURATION, SHELL_EASE, gsap, useGSAP } from "@/lib/gsap";

declare global {
  interface Window {
    __oecaIntro?: boolean;
  }
}

// Wide screens open into Home; narrow screens into Music, which stays legible.
const DESKTOP = [SHOTS.collapsed, SHOTS.home];
const MOBILE = [SHOTS.collapsed, SHOTS.music];
const WIDE = "(min-width: 768px)";

export function Hero() {
  const root = useRef<HTMLElement>(null);
  const played = useRef(false);
  const [open, setOpen] = useState(true);

  const stageNow = () => {
    const wide = matchMedia(WIDE).matches;
    const shots = wide ? DESKTOP : MOBILE;
    const el = root.current!.querySelector<HTMLElement>(`[data-stage=${wide ? "wide" : "narrow"}]`)!;
    const [closed, opened] = el.querySelectorAll<HTMLImageElement>("img");
    return { shots, closed, opened, w: stageWidth(shots) };
  };

  /** Open or close the notch, starting from whatever is on screen. */
  const shape = (toOpen: boolean, animate: boolean) => {
    const { shots, closed, opened, w } = stageNow();
    const [c, o] = shots;
    const d = animate ? SHELL_DURATION : 0;
    gsap.killTweensOf([closed, opened]);
    if (toOpen) {
      gsap.set(opened, { opacity: 1 });
      gsap.to(opened, { "--clip": clipFull(), duration: d, ease: SHELL_EASE });
      gsap.to(closed, { opacity: 0, duration: d * 0.25, ease: "none" });
    } else {
      gsap.set(closed, { opacity: 1 });
      gsap.to(opened, { "--clip": clipToward(o, c, w), duration: d, ease: SHELL_EASE });
      gsap.to(opened, { opacity: 0, duration: d * 0.3, delay: d * 0.7, ease: "none" });
    }
  };

  const { contextSafe } = useGSAP(
    () => {
      // An explicit clip on the open captures, so later tweens have a start value
      gsap.set("[data-stage] img:last-child", { "--clip": clipFull() });

      const mm = gsap.matchMedia();
      mm.add({ wide: WIDE, ok: MOTION_OK }, (ctx) => {
        if (!ctx.conditions?.ok || played.current) return;
        played.current = true;

        const { shots, closed, opened, w } = stageNow();
        const intro = root.current!.querySelectorAll("[data-intro]");
        const text = root.current!.querySelectorAll("[data-intro-text]");
        const stage = root.current!.querySelector("[data-intro-stage]");

        // One orchestrated moment: the closed notch arrives at the top edge,
        // the headline settles, then the notch opens into a real space.
        // Nothing is written before the first frame (immediateRender: false),
        // so if frames never come, the gate's fallback can still show it all.
        const from = { immediateRender: false };
        gsap
          .timeline({
            onStart: () => {
              window.__oecaIntro = true;
            },
            onComplete: () => {
              document.documentElement.classList.remove("motion");
              gsap.set(intro, { clearProps: "opacity,visibility,transform" });
            },
          })
          .set(opened, { opacity: 1, "--clip": clipToward(shots[1], shots[0], w) }, 0)
          .set(closed, { opacity: 1 }, 0)
          .fromTo(stage, { autoAlpha: 0, y: -10, ...from }, { autoAlpha: 1, y: 0, duration: 0.7 }, 0)
          .fromTo(
            text,
            { autoAlpha: 0, y: 18, ...from },
            { autoAlpha: 1, y: 0, duration: 0.9, stagger: 0.08 },
            0.2,
          )
          .to(opened, { "--clip": clipFull(), duration: SHELL_DURATION, ease: SHELL_EASE }, 1.05)
          .to(closed, { opacity: 0, duration: 0.2, ease: "none" }, 1.05);
      });
    },
    { scope: root },
  );

  const toggle = () => {
    const next = !open;
    setOpen(next);
    contextSafe(() => shape(next, matchMedia(MOTION_OK).matches))();
  };

  const label = open ? "Close the notch" : "Open the notch";

  return (
    <section id="top" ref={root} className="pt-16">
      <div className="shell pt-3 md:pt-5">
        <div className="relative overflow-hidden rounded-[28px] bg-raised shadow-[inset_0_0_0_1px_var(--color-line)]">
          <div data-intro data-intro-stage className="px-4">
            <NotchStage
              name="wide"
              shots={DESKTOP}
              visible={1}
              preload
              sizes="724px"
              className="hidden cursor-pointer md:block"
              onClick={toggle}
              aria-label={label}
              aria-pressed={open}
            />
            <NotchStage
              name="narrow"
              shots={MOBILE}
              visible={1}
              preload
              sizes="100vw"
              className="cursor-pointer md:hidden"
              onClick={toggle}
              aria-label={label}
              aria-pressed={open}
            />
          </div>

          <div className="px-5 pt-10 pb-14 text-center sm:px-8 md:pt-12 md:pb-20">
            <h1 data-intro data-intro-text className="t-display mx-auto max-w-[17ch]">
              The top of your screen, put to work.
            </h1>
            <p data-intro data-intro-text className="t-lead mx-auto mt-6 max-w-[33rem]">
              A native notch for Windows. The time, what&rsquo;s playing, a countdown and your clipboard
              history, one click away.
            </p>
            <div
              data-intro
              data-intro-text
              className="mt-9 flex flex-col items-center justify-center gap-3 sm:flex-row"
            >
              <a className="btn btn-primary w-full sm:w-auto" href={DOWNLOAD_URL}>
                <DownloadSimple size={18} weight="bold" aria-hidden />
                Download for Windows
              </a>
              <a className="btn btn-ghost w-full sm:w-auto" href="#spaces">
                Explore Oeca
              </a>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
