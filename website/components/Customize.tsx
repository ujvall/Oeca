// Every option named here exists in the app: space.rs (OecaSettings),
// config.rs (AccentChoice, ClipboardCapacity, CollapseDelay), startup.rs.
export function Customize() {
  return (
    <section className="section" aria-labelledby="customize-title">
      <div className="shell grid gap-6 md:grid-cols-12 md:gap-10">
        <h2 id="customize-title" data-reveal className="t-title max-w-[11ch] md:col-span-5">
          Set it up the way you work
        </h2>
        <p data-reveal className="t-lead max-w-xl md:col-span-7">
          Accent colour, always on top, reduced motion, start on launch, clipboard size and closing delay.
          Change them in the notch or in Settings, and Oeca keeps them.
        </p>
      </div>
    </section>
  );
}
