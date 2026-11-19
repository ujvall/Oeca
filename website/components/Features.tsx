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
    <section id="features" className="section" aria-labelledby="features-title">
      <div className="shell grid gap-12 md:grid-cols-12 md:gap-10">
        <h2 id="features-title" data-reveal className="t-title max-w-[11ch] md:col-span-5 md:self-start md:sticky md:top-28">
          Three spaces, one click apart
        </h2>
        <ul className="md:col-span-7">
          {FEATURES.map((f) => (
            <li
              key={f.title}
              data-reveal
              className="grid grid-cols-[2.5rem_1fr] gap-x-4 border-t border-line py-7 first:border-t-0 first:pt-0 sm:grid-cols-[2.5rem_9rem_1fr]"
            >
              <f.icon size={24} weight="duotone" aria-hidden className="mt-0.5 text-accent/70" />
              <h3 className="text-[1.375rem] font-semibold tracking-[-0.03em]">{f.title}</h3>
              <p className="t-body col-start-2 mt-1.5 max-w-[34ch] sm:col-start-3 sm:mt-1">{f.body}</p>
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}
