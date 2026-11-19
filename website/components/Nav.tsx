"use client";

import Image from "next/image";
import { usePathname } from "next/navigation";
import { GithubLogo } from "@phosphor-icons/react";
import { REPO_URL } from "@/lib/site";

// Section links point at the home page, so they work from every route. Plain
// anchors: static pages need no client router or RSC prefetching.
const LINKS = [
  { href: "/#features", label: "Features", wide: true },
  { href: "/changelog/", label: "Changelog", wide: false },
];

/** The navigation is the notch: true black, hanging from the lit top edge of
 *  the page, fixed while the page scrolls under it. */
export function Nav() {
  const path = usePathname();

  return (
    <header className="pointer-events-none fixed inset-x-0 top-0 z-40 flex justify-center">
      <nav aria-label="Main" className="notch pointer-events-auto flex h-14 items-center gap-1 pr-2 pl-4 sm:gap-2">
        {/* eslint-disable-next-line @next/next/no-html-link-for-pages -- full page loads by design (see LINKS) */}
        <a href="/" className="mr-2 flex items-center gap-2 rounded-xl py-1.5 pr-1.5 sm:mr-3" aria-label="Oeca home">
          <Image src="/media/oeca-icon-64.png" alt="" width={22} height={22} className="size-[22px]" />
          <span className="text-[1rem] font-semibold tracking-[-0.03em]">Oeca</span>
        </a>

        <ul className="flex items-center text-[0.875rem] text-muted">
          {LINKS.map((l) => {
            const current = path === l.href || path === `${l.href}/`;
            return (
              <li key={l.href} className={l.wide ? "hidden sm:list-item" : undefined}>
                <a
                  href={l.href}
                  aria-current={current ? "page" : undefined}
                  className={`rounded-xl px-3 py-2 transition-colors hover:bg-white/[0.06] hover:text-paper ${current ? "text-paper" : ""}`}
                >
                  {l.label}
                </a>
              </li>
            );
          })}
        </ul>

        <a
          href={REPO_URL}
          aria-label="Oeca on GitHub"
          className="grid size-9 place-items-center rounded-xl text-muted transition-colors hover:bg-white/[0.06] hover:text-paper"
        >
          <GithubLogo size={18} weight="duotone" aria-hidden />
        </a>
        {/* eslint-disable-next-line @next/next/no-html-link-for-pages -- full page loads by design (see LINKS) */}
        <a href="/#download" className="btn btn-primary btn-sm ml-1">
          Download
        </a>
      </nav>
    </header>
  );
}
