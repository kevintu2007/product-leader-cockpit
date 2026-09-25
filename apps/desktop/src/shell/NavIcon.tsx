import type { RouteId } from "./routes";

/**
 * The rail's line icons, drawn on one 18px grid with one stroke weight so
 * no destination reads louder than another (App Shell v2, concept B). They
 * are decorative: every destination's name is always in the DOM, visible
 * whenever the rail is expanded, so the icon never carries meaning alone.
 */
export function NavIcon({ routeId }: { readonly routeId: RouteId }) {
  return (
    <svg className="pmc-nav-glyph" viewBox="0 0 18 18" aria-hidden="true" focusable="false">
      {PATHS[routeId]}
    </svg>
  );
}

const PATHS: Record<RouteId, React.ReactNode> = {
  "executive-cockpit": (
    <>
      <circle cx="9" cy="9" r="6.5" />
      <circle cx="9" cy="9" r="1.6" />
    </>
  ),
  portfolio: (
    <>
      <rect x="2.5" y="2.5" width="5.5" height="5.5" />
      <rect x="10" y="2.5" width="5.5" height="5.5" />
      <rect x="2.5" y="10" width="5.5" height="5.5" />
      <rect x="10" y="10" width="5.5" height="5.5" />
    </>
  ),
  "work-queue": (
    <>
      <line x1="2.5" y1="4.5" x2="15.5" y2="4.5" />
      <line x1="2.5" y1="9" x2="15.5" y2="9" />
      <line x1="2.5" y1="13.5" x2="15.5" y2="13.5" />
    </>
  ),
  "reviews-and-reports": (
    <>
      <rect x="3" y="2.5" width="12" height="13" />
      <line x1="6" y1="6.5" x2="12" y2="6.5" />
      <line x1="6" y1="10" x2="12" y2="10" />
    </>
  ),
  "product-vault": (
    <>
      <rect x="2.5" y="2.5" width="13" height="13" />
      <circle cx="9" cy="9" r="2.6" />
    </>
  ),
  people: (
    <>
      <circle cx="6.8" cy="6.4" r="3" />
      <circle cx="12.4" cy="7.8" r="2.2" />
      <line x1="2.5" y1="14.6" x2="15.5" y2="14.6" />
    </>
  ),
  settings: (
    <>
      <circle cx="9" cy="9" r="3" />
      <line x1="9" y1="1.6" x2="9" y2="4" />
      <line x1="9" y1="14" x2="9" y2="16.4" />
      <line x1="1.6" y1="9" x2="4" y2="9" />
      <line x1="14" y1="9" x2="16.4" y2="9" />
    </>
  ),
  "system-health": <polyline points="1.5,9.5 5,9.5 7,4.5 10.5,14 12.5,9.5 16.5,9.5" />,
};
