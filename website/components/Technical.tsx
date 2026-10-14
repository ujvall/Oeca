import {
  AppWindow,
  ArrowsClockwise,
  FolderSimple,
  PaintBrushBroad,
  WaveSine,
} from "@phosphor-icons/react/dist/ssr";
import { REPO_URL } from "@/lib/site";

// Each fact restates the implementation (window.rs, renderer.rs, config.rs,
// clipboard_store.rs, settings_store.rs) as described in the README.
const FACTS = [
  {
    icon: AppWindow,
    title: "One layered window",
    body: "A per-monitor DPI-aware Win32 window. Clicks outside the notch’s shape pass through to the windows below.",
  },
  {
    icon: PaintBrushBroad,
    title: "Drawn with Direct2D",
    body: "Direct2D and DirectWrite draw into a premultiplied 32-bit bitmap that is composited with UpdateLayeredWindow. Brushes, text formats and artwork are cached per DPI.",
  },
  {
    icon: ArrowsClockwise,
    title: "Event-driven",
    body: "A blocking message loop. Clipboard changes, media sessions and the tray arrive as messages, with no polling. With nothing playing or moving, Oeca wakes only for the clock’s once-a-minute update.",
  },
  {
    icon: WaveSine,
    title: "Spring motion",
    body: "Opening, closing and switching spaces run on critically damped springs that keep their velocity when interrupted. With Reduced motion on, they snap.",
  },
  {
    icon: FolderSimple,
    title: "Where your data lives",
    body: "Settings are a small file in %LOCALAPPDATA%\\Oeca. Clipboard history is saved beside it and tied to your Windows sign-in, so a new session starts empty.",
  },
];

const STACK = ["Rust 2024", "Win32", "Direct2D", "DirectWrite", "Windows media controls"];

export function Technical() {
  return (
    <section id="built" className="pt-32 md:pt-44" aria-labelledby="built-title">
      <div className="shell grid gap-14 lg:grid-cols-12 lg:gap-10">
        <div className="lg:col-span-5">
          <div className="lg:sticky lg:top-28">
            <h2 id="built-title" className="t-title max-w-[12ch]">
              Built the Windows way.
            </h2>
            <p className="t-lead mt-6 max-w-md">
              Oeca is a single Rust executable. Its only dependencies are Microsoft&rsquo;s windows and
              windows-core crates: no runtime, no UI framework, no web view.
            </p>
            <ul
              aria-label="Technologies"
              className="mt-8 flex flex-wrap gap-x-5 gap-y-2 text-[0.9375rem] text-paper"
            >
              {STACK.map((s) => (
                <li key={s}>{s}</li>
              ))}
            </ul>
            <a href={REPO_URL} className="link mt-8 inline-block text-[0.9375rem]">
              Read the source on GitHub
            </a>
          </div>
        </div>

        <ul className="grid gap-11 lg:col-span-6 lg:col-start-7">
          {FACTS.map((f) => (
            <li key={f.title} className="flex gap-5">
              <span className="squircle">
                <f.icon size={20} aria-hidden />
              </span>
              <div>
                <h3 className="t-heading text-[1.1875rem]">{f.title}</h3>
                <p className="t-body mt-2">{f.body}</p>
              </div>
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}
