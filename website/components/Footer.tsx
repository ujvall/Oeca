import { LICENSE_URL, RELEASES_URL, REPO_URL } from "@/lib/site";

const LINKS = [
  { href: REPO_URL, label: "GitHub" },
  { href: "/changelog/", label: "Changelog" },
  { href: RELEASES_URL, label: "Releases" },
  { href: "/privacy/", label: "Privacy" },
  { href: "/code-signing/", label: "Code signing" },
  { href: LICENSE_URL, label: "License" },
];

export function Footer() {
  return (
    <footer className="border-t border-line">
      <div className="shell flex flex-col gap-6 py-10 md:flex-row md:items-center md:justify-between">
        <p className="t-caption">&copy; 2026 Ujval Sai. Free and open source under the MIT License.</p>
        <nav aria-label="Footer">
          <ul className="flex flex-wrap gap-x-6 gap-y-2 text-[0.875rem] text-muted">
            {LINKS.map((l) => (
              <li key={l.label}>
                <a className="transition-colors hover:text-paper" href={l.href}>
                  {l.label}
                </a>
              </li>
            ))}
          </ul>
        </nav>
      </div>
    </footer>
  );
}
