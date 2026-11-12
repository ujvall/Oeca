// Each fact restates the implementation (window.rs, renderer.rs, the message
// loop, settings_store.rs, clipboard_store.rs) as described in the README.
const FACTS = [
  { title: "Native", body: "One Rust executable on Win32. No runtime, UI framework or web view." },
  { title: "Direct2D", body: "Drawn with Direct2D and DirectWrite, with resources cached per DPI." },
  { title: "Event-driven", body: "No polling. With nothing playing, it wakes once a minute for the clock." },
  { title: "Local", body: "Settings and history stay on your PC. No network requests unless you check for updates." },
];

export function Technical() {
  return (
    <section id="built" className="section" aria-labelledby="built-title">
      <div className="shell">
        <h2 id="built-title" data-reveal className="t-title max-w-[16ch]">
          Built the Windows way
        </h2>
        <dl className="mt-12 grid gap-x-10 gap-y-10 sm:grid-cols-2 lg:grid-cols-4">
          {FACTS.map((f) => (
            <div key={f.title} data-reveal>
              <dt className="t-heading">{f.title}</dt>
              <dd className="t-body mt-2">{f.body}</dd>
            </div>
          ))}
        </dl>
      </div>
    </section>
  );
}
