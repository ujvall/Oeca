"use client";

import Image from "next/image";
import { useRef } from "react";
import { DownloadSimple } from "@phosphor-icons/react";
import { DOWNLOAD_URL, LICENSE_URL, RELEASES_URL, REPO_URL, REQUIREMENTS, VERSION } from "@/lib/site";
import { MOTION_OK, gsap, useGSAP } from "@/lib/gsap";

export function Download() {
  const root = useRef<HTMLElement>(null);

  // The icon settles into place as the section arrives (transform only).
  useGSAP(
    () => {
      gsap.matchMedia().add(MOTION_OK, () => {
        gsap.from("[data-download-icon]", {
          y: 28,
          scale: 0.92,
          filter: "blur(10px)",
          autoAlpha: 0,
          clearProps: "filter",
          duration: 1.1,
          scrollTrigger: { trigger: root.current, start: "top 75%", once: true },
        });
      });
    },
    { scope: root },
  );

  return (
    <section id="download" ref={root} className="section" aria-labelledby="download-title">
      <div className="shell">
        <div className="grid gap-10 md:grid-cols-12 md:items-end">
          <div className="md:col-span-7">
            <Image
              data-download-icon
              src="/media/oeca-icon-256.webp"
              alt=""
              width={96}
              height={96}
              className="size-16 md:size-20"
            />
            <h2 id="download-title" className="t-title mt-8">
              Get Oeca for Windows
            </h2>
            <p className="t-lead mt-5 max-w-lg">
              <span className="t-mono text-[0.95em] text-paper">v{VERSION}</span>. Free and open source under
              the{" "}
              <a className="link" href={LICENSE_URL}>
                MIT License
              </a>
              .
            </p>
          </div>
          <div className="flex flex-col gap-3 sm:flex-row md:col-span-5 md:justify-end">
            <a className="btn btn-primary group w-full sm:w-auto" href={DOWNLOAD_URL}>
              <DownloadSimple
                size={18}
                weight="bold"
                aria-hidden
                className="transition-transform duration-200 ease-out group-hover:translate-y-0.5"
              />
              Download for Windows
            </a>
            <a className="btn btn-ghost w-full sm:w-auto" href={RELEASES_URL}>
              All releases
            </a>
          </div>
        </div>

        <dl className="mt-16 grid gap-10 sm:grid-cols-3">
          <div data-reveal>
            <dt className="text-[0.9375rem] font-medium">Requirements</dt>
            <dd className="t-body mt-1.5 text-[0.9375rem]">{REQUIREMENTS}.</dd>
          </div>
          <div data-reveal>
            <dt className="text-[0.9375rem] font-medium">Installation</dt>
            <dd className="t-body mt-1.5 text-[0.9375rem]">
              Installs for your account only, with no administrator rights. Uninstall from Settings, Apps.
            </dd>
          </div>
          <div data-reveal>
            <dt className="text-[0.9375rem] font-medium">Code signing</dt>
            <dd className="t-body mt-1.5 text-[0.9375rem]">
              Not signed yet, so Windows SmartScreen may ask you to confirm. Free signing through SignPath
              Foundation is pending; see the{" "}
              <a className="link" href="/code-signing/">
                Code signing policy
              </a>
              .
            </dd>
          </div>
        </dl>
        <p className="t-caption mt-10 max-w-3xl">
          Oeca makes no network requests unless you check for updates. Read the{" "}
          <a className="link" href="/privacy/">
            Privacy policy
          </a>{" "}
          and the{" "}
          <a className="link" href="/code-signing/">
            Code signing policy
          </a>
          , or browse the{" "}
          <a className="link" href={REPO_URL}>
            source on GitHub
          </a>
          .
        </p>
      </div>
    </section>
  );
}
