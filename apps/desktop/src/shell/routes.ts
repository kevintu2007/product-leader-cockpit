/**
 * Frozen route registry, matching the DG3 UI
 * Contract's Route Contract table and Information Architecture section
 * exactly: docs/ui-contract.md
 *
 * This registry owns navigation identity only -- route IDs, labels, and
 * primary-nav order. Route *content* (S01-S11) belongs to each route's own
 * module; this registry owns only the shared shell that hosts them.
 *
 * Labels stay in the canonical English form the Route Contract table
 * itself uses (Executive Cockpit, Portfolio, ...): the design system's
 * "Content and naming" section reserves untranslated English for canonical
 * product/domain identifiers, and every one of these seven destinations is
 * already used as a domain identifier throughout docs/domain-glossary.md and the DG0/DG3
 * specs -- not a caption to translate.
 */

export type RouteId =
  | "executive-cockpit"
  | "portfolio"
  | "work-queue"
  | "reviews-and-reports"
  | "product-vault"
  | "people"
  | "settings"
  | "system-health";

export interface RouteDefinition {
  readonly id: RouteId;
  readonly label: string;
  /** 1-7 for the seven stable primary destinations; absent for System
   * Health, which is reachable as a system destination but does not
   * displace the seven primary destinations (DG3 Information Architecture). */
  readonly primaryOrder?: number;
}

export const ROUTES: readonly RouteDefinition[] = [
  { id: "executive-cockpit", label: "Executive Cockpit", primaryOrder: 1 },
  { id: "portfolio", label: "Portfolio", primaryOrder: 2 },
  { id: "work-queue", label: "Work Queue", primaryOrder: 3 },
  { id: "reviews-and-reports", label: "Reviews & Reports", primaryOrder: 4 },
  { id: "product-vault", label: "Product Vault", primaryOrder: 5 },
  { id: "people", label: "People", primaryOrder: 6 },
  { id: "settings", label: "Settings", primaryOrder: 7 },
  { id: "system-health", label: "System Health" },
];

export const DEFAULT_ROUTE_ID: RouteId = "executive-cockpit";

export const PRIMARY_ROUTES: readonly RouteDefinition[] = ROUTES.filter(
  (route): route is RouteDefinition & { primaryOrder: number } => route.primaryOrder !== undefined,
).sort((a, b) => a.primaryOrder - b.primaryOrder);

export function findRoute(id: RouteId): RouteDefinition {
  const route = ROUTES.find((candidate) => candidate.id === id);
  if (!route) {
    throw new Error(`Unknown route id: ${id}`);
  }
  return route;
}

const PRODUCT_NAME = "Product Mission Control";

/**
 * DG3 Accessibility and Interaction Contract: "Every route sets a distinct
 * document title beginning with the current route name and Product Mission
 * Control, so Windows task switching and assistive technology can
 * distinguish open product surfaces."
 */
export function documentTitleFor(id: RouteId): string {
  return `${findRoute(id).label} – ${PRODUCT_NAME}`;
}
