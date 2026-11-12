/** The current Windows mark: four equal squares. Phosphor's WindowsLogo is
 *  the older, slanted one. */
export function WindowsMark({ size = 16 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="currentColor" aria-hidden>
      <rect x="0" y="0" width="7.5" height="7.5" />
      <rect x="8.5" y="0" width="7.5" height="7.5" />
      <rect x="0" y="8.5" width="7.5" height="7.5" />
      <rect x="8.5" y="8.5" width="7.5" height="7.5" />
    </svg>
  );
}
