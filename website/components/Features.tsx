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
        <h2 id="features-title" data-reveal className="t-title max-w-[11ch] md:col-span-5">
          Three spaces, one click apart
        </h2>
        <ul className="grid gap-x-10 gap-y-10 sm:grid-cols-2 md:col-span-7">
          {FEATURES.map((f) => (
            <li key={f.title} data-reveal className="group">
              <span className="icon-tile">
                <f.icon size={20} weight="duotone" aria-hidden />
              </span>
              <h3 className="t-heading mt-5">{f.title}</h3>
              <p className="t-body mt-2 max-w-[30ch]">{f.body}</p>
            </li>
          ))}
        </ul>
      </div>
    </section>
  );
}
