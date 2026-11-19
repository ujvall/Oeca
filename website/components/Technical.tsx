import { Cpu, Lightning, LockSimple, PaintBrush } from "@phosphor-icons/react/dist/ssr";

// Each fact restates the implementation (window.rs, renderer.rs, the message
// loop, settings_store.rs, clipboard_store.rs) as described in the README.
const FACTS = [
  { icon: Cpu, title: "Native", body: "One Rust executable on Win32. No runtime, UI framework or web view." },
  { icon: PaintBrush, title: "Direct2D", body: "Drawn with Direct2D and DirectWrite, with resources cached per DPI." },
  { icon: Lightning, title: "Event-driven", body: "No polling. With nothing playing, it wakes once a minute for the clock." },
  { icon: LockSimple, title: "Local", body: "Settings and history stay on your PC. No network requests unless you check for updates." },
];

export function Technical() {
  return (
    <section id="built" className="section" aria-labelledby="built-title">
      <div className="shell">
        <h2 id="built-title" data-reveal className="t-title">
          Built the Windows way
        </h2>
        <ul
          data-reveal
          className="panel mt-8 grid divide-y divide-line sm:grid-cols-2 sm:divide-y-0 lg:grid-cols-4 lg:divide-x"
        >
          {FACTS.map((f) => (
            <li key={f.title} className="p-6">
              <f.icon size={20} aria-hidden className="text-muted" />
              <h3 className="t-heading mt-4">{f.title}</h3>
              <p className="t-body mt-1.5">{f.body}</p>
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}
