"use client";

import { useEffect } from "react";
import Lenis from "lenis";
import "lenis/dist/lenis.css";
import { MOTION_OK, ScrollTrigger, gsap } from "@/lib/gsap";

declare global {
  interface Window {
    __oecaIntro?: boolean;
  }
}

/** Page-wide motion: Lenis smooth scrolling driven by GSAP's ticker, smooth
 *  same-page anchor jumps, and a blur-in reveal for every `[data-reveal]`.
 *  Under reduced motion none of it runs: native scrolling, nothing hidden. */
export function Motion() {
  useEffect(() => {
    const mm = gsap.matchMedia();

    mm.add(MOTION_OK, () => {
      const lenis = new Lenis({ lerp: 0.1, wheelMultiplier: 0.9 });
      lenis.on("scroll", ScrollTrigger.update);
      const tick = (t: number) => lenis.raf(t * 1000);
      gsap.ticker.add(tick);
      gsap.ticker.lagSmoothing(0);

      // Same-page anchors glide instead of jumping; the nav's height is kept clear.
      const onClick = (e: MouseEvent) => {
        const a = (e.target as Element).closest<HTMLAnchorElement>("a[href*='#']");
        if (!a || e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey) return;
        const url = new URL(a.href);
        if (url.pathname !== location.pathname || !url.hash) return;
        const target = document.querySelector<HTMLElement>(url.hash);
        if (!target) return;
        e.preventDefault();
        history.pushState(null, "", url.hash);
        lenis.scrollTo(target, { offset: -72, duration: 1.1 });
      };
      document.addEventListener("click", onClick);

      // The hero settles on load: hidden before first paint by the gate in
      // layout.tsx, then the same blur-in as the rest of the page.
      const intro = gsap.utils.toArray<HTMLElement>("[data-intro]");
      gsap.fromTo(
        intro,
        { autoAlpha: 0, y: 16, filter: "blur(8px)", willChange: "transform, opacity, filter" },
        {
          autoAlpha: 1,
          y: 0,
          filter: "blur(0px)",
          duration: 1,
          delay: 0.05,
          stagger: 0.08,
          ease: "power3.out",
          onStart: () => {
            window.__oecaIntro = true;
          },
          onComplete: () => {
            document.documentElement.classList.remove("motion");
            gsap.set(intro, { clearProps: "opacity,visibility,transform,filter,willChange" });
          },
        },
      );

      // Content arrives out of focus and settles: blur, a short rise, opacity.
      // Hidden items wait on opacity alone (a resting blur filter costs paint);
      // each one gets its own layer only while it animates, so the blur runs
      // on the compositor instead of repainting under the scroll.
      const items = gsap.utils.toArray<HTMLElement>("[data-reveal]");
      gsap.set(items, { autoAlpha: 0 });
      ScrollTrigger.batch(items, {
        start: "top 92%",
        once: true,
        onEnter: (batch) =>
          gsap.fromTo(
            batch,
            { y: 20, filter: "blur(6px)", willChange: "transform, opacity, filter" },
            {
              autoAlpha: 1,
              y: 0,
              filter: "blur(0px)",
              duration: 0.7,
              stagger: 0.06,
              ease: "power3.out",
              clearProps: "filter,transform,willChange",
            },
          ),
      });

      return () => {
        document.removeEventListener("click", onClick);
        gsap.ticker.remove(tick);
        lenis.destroy();
      };
    });

    return () => mm.revert();
  }, []);

  return null;
}
