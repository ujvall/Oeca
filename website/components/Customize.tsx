import { Clock, Palette, Power, PushPin, Stack, Wind } from "@phosphor-icons/react/dist/ssr";

// Every option named here exists in the app: space.rs (OecaSettings),
// config.rs (AccentChoice, ClipboardCapacity, CollapseDelay), startup.rs.
const OPTIONS = [
  { icon: Palette, label: "Accent colour" },
  { icon: PushPin, label: "Always on top" },
  { icon: Wind, label: "Reduced motion" },
  { icon: Power, label: "Start on launch" },
  { icon: Stack, label: "Clipboard size" },
  { icon: Clock, label: "Closing delay" },
];

export function Customize() {
  return (
    <section className="section" aria-labelledby="customize-title">
      <div className="shell grid gap-8 md:grid-cols-12 md:gap-10">
        <h2 id="customize-title" data-reveal className="t-title max-w-[12ch] md:col-span-4">
          Set it up the way you work
        </h2>
        <div data-reveal className="md:col-span-8">
          <p className="t-body max-w-xl">
            Change them in the notch or in Settings, and Oeca keeps them for next time.
          </p>
          <ul aria-label="Settings" className="mt-6 flex flex-wrap gap-2">
            {OPTIONS.map((o) => (
              <li key={o.label} className="chip">
                <o.icon size={16} aria-hidden />
                {o.label}
              </li>
            ))}
          </ul>
        </div>
      </div>
    </section>
  );
}
