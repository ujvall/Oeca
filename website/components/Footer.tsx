import Image from "next/image";
import { LICENSE_URL, RELEASES_URL, REPO_URL } from "@/lib/site";

export function Footer() {
  return (
    <footer className="border-t border-line">
      <div className="shell flex flex-col gap-8 py-10 md:flex-row md:items-start md:justify-between">
        <div>
          <div className="flex items-center gap-2.5">
            <Image src="/media/oeca-icon-64.png" alt="" width={22} height={22} className="size-[22px]" />
            <span className="text-[1.0625rem] font-semibold tracking-[-0.02em]">Oeca</span>
          </div>
          <p className="t-caption mt-4 max-w-md">
            &copy; 2026 Ujval Sai. Oeca is source-available, proprietary software; its code is public to read,
            not to reuse without permission. Third-party materials keep their own licences: the site and
            wordmark are set in General Sans by the Indian Type Foundry, via Fontshare, and the in-app icons
            in the captures are{" "}
            <a className="link" href="https://www.flaticon.com/uicons">
              designed by UIcons from Flaticon
            </a>
            .
          </p>
        </div>
        <nav aria-label="Footer">
          <ul className="flex flex-wrap gap-x-6 gap-y-3 text-[0.9063rem] text-muted">
            <li>
              <a className="transition-colors hover:text-paper" href={REPO_URL}>
                GitHub
              </a>
            </li>
            <li>
              <a className="transition-colors hover:text-paper" href="/changelog/">
                Changelog
              </a>
            </li>
            <li>
              <a className="transition-colors hover:text-paper" href={RELEASES_URL}>
                Releases
              </a>
            </li>
            <li>
              <a className="transition-colors hover:text-paper" href={LICENSE_URL}>
                License
              </a>
            </li>
          </ul>
        </nav>
      </div>
    </footer>
  );
}
