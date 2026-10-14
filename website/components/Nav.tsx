"use client";

import Image from "next/image";
import { usePathname } from "next/navigation";
import { useRef } from "react";
import { GithubLogo } from "@phosphor-icons/react";
import { REPO_URL } from "@/lib/site";
import { ScrollTrigger, useGSAP } from "@/lib/gsap";

// Section links point at the home page, so they work from every route. Plain
// anchors: two static pages need no client router or RSC prefetching.
const LINKS = [
  { href: "/#spaces", label: "Spaces", wide: true },
  { href: "/#built", label: "Under the hood", wide: true },
  { href: "/changelog/", label: "Changelog", wide: false },
];

export function Nav() {
  const ref = useRef<HTMLElement>(null);
  const path = usePathname();

  // Once the page moves, the bar gains a solid backing and a hairline so links
  // stay legible over content (a class toggle; no scroll listener).
  useGSAP(
    () => {
      ScrollTrigger.create({
        start: 8,
        end: "max",
        toggleClass: { targets: ref.current, className: "is-scrolled" },
      });
    },
    { scope: ref },
  );

  return (
    <header
      ref={ref}
      className="group fixed inset-x-0 top-0 z-40 border-b border-transparent transition-[background-color,border-color] duration-300 [&.is-scrolled]:border-line [&.is-scrolled]:bg-ink/85 [&.is-scrolled]:backdrop-blur-md"
    >
      <nav aria-label="Main" className="shell flex h-16 items-center gap-4 sm:gap-6">
        {/* eslint-disable-next-line @next/next/no-html-link-for-pages -- full page loads by design (see LINKS) */}
        <a href="/" className="-m-2 flex items-center gap-2.5 rounded-lg p-2" aria-label="Oeca home">
          <Image src="/media/oeca-icon-64.png" alt="" width={26} height={26} className="size-[26px]" />
          <span className="text-[1.1875rem] font-semibold tracking-[-0.02em]">Oeca</span>
        </a>

        <ul className="ml-auto flex items-center gap-7 text-[0.9063rem] text-muted">
          {LINKS.map((l) => {
            const current = path === l.href || path === `${l.href}/`;
            return (
              <li key={l.href} className={l.wide ? "hidden sm:list-item" : undefined}>
                <a
                  href={l.href}
                  aria-current={current ? "page" : undefined}
                  className={`transition-colors hover:text-paper ${current ? "text-paper" : ""}`}
                >
                  {l.label}
                </a>
              </li>
            );
          })}
        </ul>

        <div className="flex items-center gap-2">
          <a
            href={REPO_URL}
            className="grid size-9 place-items-center rounded-full text-muted transition-colors hover:bg-white/5 hover:text-paper"
            aria-label="Oeca on GitHub"
          >
            <GithubLogo size={20} weight="regular" aria-hidden />
          </a>
          {/* eslint-disable-next-line @next/next/no-html-link-for-pages -- full page loads by design (see LINKS) */}
          <a href="/#download" className="btn btn-primary btn-sm">
            Download
          </a>
        </div>
      </nav>
    </header>
  );
}
