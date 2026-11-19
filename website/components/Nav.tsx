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

/** A straight, full-width bar: opaque enough to read over the grid, with a
 *  hairline underneath. */
export function Nav() {
  const path = usePathname();

  return (
    <header className="fixed inset-x-0 top-0 z-40 border-b border-line bg-ink/85 backdrop-blur-md">
      <nav aria-label="Main" className="shell flex h-14 items-center gap-6">
        {/* eslint-disable-next-line @next/next/no-html-link-for-pages -- full page loads by design (see LINKS) */}
        <a href="/" className="-m-1.5 flex items-center gap-2 rounded-lg p-1.5" aria-label="Oeca home">
          <Image src="/media/oeca-icon-64.png" alt="" width={22} height={22} className="size-[22px]" />
          <span className="text-[0.9375rem] font-semibold tracking-[-0.02em]">Oeca</span>
        </a>

        <ul className="ml-auto flex items-center gap-6 text-[0.875rem] text-muted">
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
            aria-label="Oeca on GitHub"
            className="grid size-9 place-items-center rounded-lg text-muted transition-colors hover:bg-white/[0.06] hover:text-paper"
          >
            <GithubLogo size={18} aria-hidden />
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
