"use client";

import Image from "next/image";
import { useRef } from "react";
import { SHOTS } from "@/lib/site";
import { MOTION_OK, gsap, useGSAP } from "@/lib/gsap";

const shot = SHOTS.collapsedTimer;

export function Glance() {
  const root = useRef<HTMLElement>(null);

  // A slow push-in as the panel arrives: the eye is led to the details of the
  // closed notch. Transform only, scrubbed, so it is never hidden or stuck.
  useGSAP(
    () => {
      gsap.matchMedia().add(MOTION_OK, () => {
        gsap.fromTo(
          "[data-glance-shot]",
          { scale: 0.86, y: -6 },
          {
            scale: 1,
            y: 0,
            ease: "none",
            scrollTrigger: {
              trigger: "[data-glance-panel]",
              start: "top 90%",
              end: "center 55%",
              scrub: 0.6,
            },
          },
        );
      });
    },
    { scope: root },
  );

  return (
    <section ref={root} className="rule" aria-labelledby="glance-title">
      <div className="shell">
      <div className="frame grid items-center gap-12 px-5 py-24 sm:px-8 md:grid-cols-12 md:gap-10 md:py-32">
        <div className="md:col-span-5">
          <p className="t-label">At a glance</p>
          <h2 id="glance-title" className="t-title mt-5 max-w-[12ch]">
            Useful even when it&rsquo;s closed
          </h2>
          <p className="t-body mt-6 max-w-md">
            The closed notch shows the time, the track&rsquo;s artwork and a visualizer. A running timer joins
            it as a ring, so you can watch it count down without opening anything.
          </p>
          <p className="t-body mt-4 max-w-md">
            Click to open. It closes as soon as you click elsewhere, or after a delay you choose.
          </p>
        </div>

        <figure className="md:col-span-7">
          <div
            data-glance-panel
            className="relative flex aspect-[2/1] items-start justify-center overflow-hidden rounded-2xl bg-raised shadow-[inset_0_0_0_1px_var(--color-line-strong)]"
          >
            <Image
              data-glance-shot
              src={shot.src}
              alt={shot.alt}
              width={Math.round(shot.w)}
              height={Math.round(shot.h)}
              sizes="(min-width: 768px) 440px, 82vw"
              className="h-auto w-[82%] max-w-[440px] origin-top"
              draggable={false}
            />
          </div>
          <figcaption className="t-caption mt-4">
            The closed notch with a five-minute timer running.
          </figcaption>
        </figure>
      </div>
      </div>
    </section>
  );
}
