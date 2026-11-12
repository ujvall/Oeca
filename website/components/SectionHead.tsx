/** Section opener: label and title on the left, the lead on the right, level
 *  with the title's last line. Stacks on small screens. */
export function SectionHead({
  id,
  label,
  title,
  children,
}: {
  id: string;
  label: string;
  title: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div className="grid gap-6 md:grid-cols-12 md:items-end md:gap-10">
      <div data-reveal className="md:col-span-6">
        <p className="t-label">{label}</p>
        <h2 id={id} className="t-title mt-4 max-w-[16ch]">
          {title}
        </h2>
      </div>
      <p data-reveal className="t-lead max-w-xl md:col-span-6 md:pb-1">
        {children}
      </p>
    </div>
  );
}
