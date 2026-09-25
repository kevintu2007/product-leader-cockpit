import { NavIcon } from "./NavIcon";
import { findRoute, PRIMARY_ROUTES, type RouteDefinition, type RouteId } from "./routes";
import { useT } from "../i18n/useT";

export interface NavigationProps {
  activeRouteId: RouteId;
  onNavigate: (id: RouteId) => void;
  /** Items that need the person's attention, per destination. A destination
   * with no entry shows no badge -- the rail never shows a zero it cannot
   * back. */
  counts?: Partial<Record<RouteId, number>> | undefined;
}

const SYSTEM_HEALTH = findRoute("system-health");

/**
 * The primary navigation rail (App Shell v2, concept B "Portfolio Lens",
 * selected by the product owner 2026-08-12).
 *
 * A compact icon rail that expands to its full labelled width on pointer
 * hover or keyboard focus, which is the design system's allowance for a
 * compact mode "only if labels remain discoverable". The seven primary
 * destinations keep their frozen order; System Health sits apart at the
 * foot so it never displaces them. Every destination is a native `<button>`
 * (Tab, Enter, Space) marked with `aria-current="page"` when selected, so
 * selection is announced and shown by a bar, weight and surface -- not by
 * colour alone.
 */
export function Navigation({ activeRouteId, onNavigate, counts = {} }: NavigationProps) {
  const t = useT();
  return (
    <nav aria-label={t("shell.nav.primary")} className="pmc-nav">
      <div className="pmc-nav-brand">
        <span className="pmc-nav-mark" aria-hidden="true">
          <span />
          <span />
          <span />
        </span>
        <span className="pmc-nav-brand-name">
          <strong>Product</strong>
          <small>Mission Control</small>
        </span>
      </div>
      <ul className="pmc-nav-list">
        {PRIMARY_ROUTES.map((route) => (
          <li key={route.id}>
            <NavButton
              route={route}
              isActive={route.id === activeRouteId}
              count={counts[route.id]}
              onNavigate={onNavigate}
            />
          </li>
        ))}
      </ul>
      <div className="pmc-nav-foot">
        <NavButton
          route={SYSTEM_HEALTH}
          isActive={activeRouteId === "system-health"}
          count={counts["system-health"]}
          onNavigate={onNavigate}
        />
      </div>
    </nav>
  );
}

function NavButton({
  route,
  isActive,
  count,
  onNavigate,
}: {
  readonly route: RouteDefinition;
  readonly isActive: boolean;
  readonly count: number | undefined;
  readonly onNavigate: (id: RouteId) => void;
}) {
  const t = useT();
  const showCount = count !== undefined && count > 0;
  return (
    <button
      type="button"
      className="pmc-nav-item"
      aria-current={isActive ? "page" : undefined}
      // The badge is decorative; its meaning travels in the name instead, so
      // a screen reader hears "Work Queue，4 件需要注意" rather than "Work
      // Queue 4".
      aria-label={
        showCount ? t.plural("shell.nav.attention", count, { route: route.label }) : undefined
      }
      data-active={isActive}
      title={route.label}
      onClick={() => {
        onNavigate(route.id);
      }}
    >
      <NavIcon routeId={route.id} />
      <span className="pmc-nav-label">{route.label}</span>
      {showCount ? (
        <span className="pmc-nav-count" aria-hidden="true">
          {String(count)}
        </span>
      ) : null}
    </button>
  );
}
