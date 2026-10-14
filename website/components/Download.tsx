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
          duration: 1.1,
          scrollTrigger: { trigger: root.current, start: "top 75%", once: true },
        });
      });
    },
    { scope: root },
  );

  return (
    <section
      id="download"
      ref={root}
      className="pt-36 pb-28 md:pt-52 md:pb-36"
      aria-labelledby="download-title"
    >
      <div className="shell text-center">
        <Image
          data-download-icon
          src="/media/oeca-icon-256.webp"
          alt=""
          width={112}
          height={112}
          className="mx-auto size-24 md:size-28"
        />
        <h2 id="download-title" className="t-title mt-10">
          Get Oeca for Windows.
        </h2>
        <p className="t-lead mx-auto mt-5 max-w-lg">
          Version {VERSION}. Free and open source under the{" "}
          <a className="link" href={LICENSE_URL}>
            MIT License
          </a>
          .
        </p>

        <div className="mt-10 flex flex-col items-center justify-center gap-3 sm:flex-row">
          <a className="btn btn-primary w-full sm:w-auto" href={DOWNLOAD_URL}>
            <DownloadSimple size={18} weight="bold" aria-hidden />
            Download for Windows
          </a>
          <a className="btn btn-ghost w-full sm:w-auto" href={RELEASES_URL}>
            All releases
          </a>
        </div>

        <dl className="mx-auto mt-20 grid max-w-4xl gap-8 border-t border-line pt-10 text-left sm:grid-cols-3">
          <div>
            <dt className="text-[0.9375rem] font-medium">Requirements</dt>
            <dd className="t-body mt-1.5 text-[0.9375rem]">{REQUIREMENTS}.</dd>
          </div>
          <div>
            <dt className="text-[0.9375rem] font-medium">Installation</dt>
            <dd className="t-body mt-1.5 text-[0.9375rem]">
              Installs for your account only, with no administrator rights. Uninstall from Settings, Apps.
            </dd>
          </div>
          <div>
            <dt className="text-[0.9375rem] font-medium">SmartScreen</dt>
            <dd className="t-body mt-1.5 text-[0.9375rem]">
              The installer is not code-signed, so Windows may ask you to confirm. The{" "}
              <a className="link" href={REPO_URL}>
                source
              </a>{" "}
              is public.
            </dd>
          </div>
        </dl>
      </div>
    </section>
  );
}
