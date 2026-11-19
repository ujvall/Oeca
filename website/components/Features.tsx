import { ClipboardText, House, MusicNotes, Timer } from "@phosphor-icons/react/dist/ssr";

// Each line restates the app: space.rs (Home, Music, Clipboard), media.rs,
// clipboard capacities (config.rs), the collapsed notch and timer ring (timer.rs).
const FEATURES = [
  { icon: House, title: "Home", body: "What’s playing, a countdown and the time, in one view." },
  { icon: MusicNotes, title: "Music", body: "Artwork and controls for any app that uses the Windows media controls." },
  { icon: ClipboardText, title: "Clipboard", body: "Your last 5, 10 or 20 copies, text and images. Click one to copy it again." },
  { icon: Timer, title: "Closed", body: "The time, artwork and a visualizer, with a ring while a timer runs." },
];

export function Features() {
  return (
    <section id="features" className="section scroll-mt-14" aria-labelledby="features-title">
      <div className="shell grid gap-8 md:grid-cols-12 md:gap-10">
        <h2 id="features-title" data-reveal className="t-title max-w-[12ch] md:col-span-4">
          Three spaces, one click apart
        </h2>
        <ul data-reveal className="panel grid sm:grid-cols-2 md:col-span-8">
          {FEATURES.map((f, i) => (
            <li
              key={f.title}
              className={`p-6 ${i > 0 ? "border-t border-line" : ""} ${i === 1 ? "sm:border-t-0 sm:border-l" : ""} ${i === 3 ? "sm:border-l" : ""}`}
            >
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
