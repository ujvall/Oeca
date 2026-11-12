import { SectionHead } from "./SectionHead";

// Every option here exists in the app: space.rs (OecaSettings), config.rs
// (AccentChoice, ClipboardCapacity, CollapseDelay), tray.rs, startup.rs.
const OPTIONS = [
  { term: "Accent colour", desc: "White, blue, teal, green, amber, rose or violet." },
  { term: "Always on top", desc: "Keep the notch above other windows, or let them cover it." },
  { term: "Reduced motion", desc: "Oeca’s own transitions become instant changes." },
  { term: "Start on launch", desc: "Off until you turn it on. Uninstalling removes it." },
  { term: "Clipboard size", desc: "Keep 5, 10 or 20 items, and choose whether clearing asks first." },
  { term: "Closing delay", desc: "Close as soon as you click elsewhere, or 5, 10 or 15 seconds after." },
  { term: "What it shows", desc: "Show or hide the visualizer and the name of the playing app." },
  { term: "Notification area", desc: "Open Oeca, Open Settings and Quit Oeca from the tray icon." },
];

export function Customize() {
  return (
    <section className="section" aria-labelledby="customize-title">
      <div className="shell">
        <SectionHead id="customize-title" label="Settings" title="Set it up the way you work">
          Change settings in the notch or in the full Settings window. Oeca saves them and restores them the
          next time it starts.
        </SectionHead>
        <dl className="mt-14 grid gap-x-10 gap-y-10 sm:grid-cols-2 lg:grid-cols-4">
          {OPTIONS.map((o) => (
            <div key={o.term} data-reveal>
              <dt className="font-medium tracking-[-0.01em]">{o.term}</dt>
              <dd className="t-body mt-2 text-[0.9375rem]">{o.desc}</dd>
            </div>
          ))}
        </dl>
      </div>
    </section>
  );
}
