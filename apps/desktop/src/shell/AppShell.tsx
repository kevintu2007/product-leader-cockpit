import { useEffect, useState, type ReactNode } from "react";

import { Navigation } from "./Navigation";
import { PolicyStrip, type PolicyStripState } from "./PolicyStrip";
import { DEFAULT_ROUTE_ID, documentTitleFor, findRoute, type RouteId } from "./routes";
import { useT } from "../i18n/useT";

export interface AppShellProps {
  /** Cross-route policy states to surface in the persistent strip. Empty by
   * default: no operation-owning slice exists yet to supply live facts. */
  readonly policyStates?: readonly PolicyStripState[];
  readonly initialRouteId?: RouteId;
  /** Route content, supplied by the route's own owning slice. Falls back to
   * a placeholder: the app shell owns the frame, not S01-S11 content. */
  readonly renderRoute?: (routeId: RouteId, navigate: (id: RouteId) => void) => ReactNode;
  /** Attention badges on the rail, supplied by whoever has read the facts. */
  readonly navCounts?: Partial<Record<RouteId, number>> | undefined;
  /** Go to a route from outside the rail (e.g. "Open Backups"). A new
   * `sequence` is a new request, even to the same route. */
  readonly routeRequest?: { readonly routeId: RouteId; readonly sequence: number } | undefined;
  /** The sample workspace's marker (item ⑨, §5), shown on every route. */
  readonly workspaceBadge?: ReactNode;
}

/**
 * The shared production shell, laid out as the
 * accepted App Shell v2 concept B: a compact navigation rail, a top bar that
 * names where the person is, the persistent cross-route policy strip, and
 * one scrolling content region. `document.title` follows the active route
 * (DG3 Accessibility and Interaction Contract: "Every route sets a distinct
 * document title beginning with the current route name").
 *
 * The route name is the page's one `h1`, set in the top bar as the current
 * segment of the breadcrumb, so route content is free to lead with its own
 * visual headline without a second top-level heading.
 *
 * Routing is a small internal state machine, not a URL-based router: v1 is
 * a single-window Windows desktop app with a frozen, small route set, and
 * the DG3 contract does not require browser history semantics.
 */
export function AppShell({
  policyStates = [],
  initialRouteId = DEFAULT_ROUTE_ID,
  renderRoute,
  navCounts,
  routeRequest,
  workspaceBadge,
}: AppShellProps) {
  const [activeRouteId, setActiveRouteId] = useState<RouteId>(initialRouteId);

  // A new request is applied while rendering (React's "adjust state when a
  // prop changes"), not in an effect: the route switches in the same pass.
  const [handledRequest, setHandledRequest] = useState<number | undefined>(undefined);
  if (routeRequest !== undefined && routeRequest.sequence !== handledRequest) {
    setHandledRequest(routeRequest.sequence);
    setActiveRouteId(routeRequest.routeId);
  }

  useEffect(() => {
    document.title = documentTitleFor(activeRouteId);
  }, [activeRouteId]);

  const route = findRoute(activeRouteId);
  const today = todayParts();

  return (
    <div className="pmc-shell">
      <Navigation activeRouteId={activeRouteId} onNavigate={setActiveRouteId} counts={navCounts} />
      <div className="pmc-shell-main">
        <header className="pmc-topbar">
          <div className="pmc-topbar-crumb">
            <span>Product Mission Control</span>
            <span aria-hidden="true">/</span>
            <h1 id="pmc-route-title" className="pmc-route-title">
              {route.label}
            </h1>
            <span className="pmc-topbar-date">{today.date}</span>
            <span className="pmc-topbar-zone">{today.zone}</span>
          </div>
          <div className="pmc-topbar-actions">
            {workspaceBadge}
            <ThemeToggle />
          </div>
        </header>
        <PolicyStrip states={policyStates} />
        <main aria-labelledby="pmc-route-title" className="pmc-shell-content">
          {/* Falls back when the prop is absent *or* when an owning slice has
              no content for this route yet, which is what the prop contract
              above already promises. */}
          {renderRoute?.(activeRouteId, setActiveRouteId) ?? <RoutePlaceholder />}
        </main>
      </div>
    </div>
  );
}

function RoutePlaceholder() {
  const t = useT();
  return (
    <div className="pmc-route-placeholder">
      <p>{t("shell.routePlaceholder")}</p>
    </div>
  );
}

/** Today in the person's own time zone, as the top bar's date segment. The
 * date and the zone stay two facts, set apart by space: joining them with a
 * dot would make one string out of two different things. */
function todayParts(): { readonly date: string; readonly zone: string } {
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  // `sv-SE` formats a calendar date as YYYY-MM-DD, the form every other
  // PMC surface uses; the zone is named so the date is never ambiguous.
  const date = new Intl.DateTimeFormat("sv-SE", { timeZone: zone }).format(new Date());
  return { date, zone };
}

type Theme = "light" | "dark";
const THEME_KEY = "pmc-theme";

function readStoredTheme(): Theme | null {
  try {
    const stored = window.localStorage.getItem(THEME_KEY);
    return stored === "light" || stored === "dark" ? stored : null;
  } catch {
    return null;
  }
}

function systemTheme(): Theme {
  // Guarded: not every host that renders the shell (a test DOM, an embedded
  // preview) implements matchMedia, and a missing API means "no preference".
  if (typeof window.matchMedia !== "function") {
    return "light";
  }
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/**
 * Light / dark. With no stored choice the app follows Windows, which is what
 * `tokens.css` already does when no `data-theme` is stamped; pressing the
 * button records an explicit choice that then wins over the OS.
 */
function ThemeToggle() {
  const t = useT();
  const [theme, setTheme] = useState<Theme | null>(readStoredTheme);

  useEffect(() => {
    if (theme === null) {
      delete document.documentElement.dataset.theme;
      return;
    }
    document.documentElement.dataset.theme = theme;
    try {
      window.localStorage.setItem(THEME_KEY, theme);
    } catch {
      // A browser that refuses storage still honours the choice for this run.
    }
  }, [theme]);

  const effective = theme ?? systemTheme();
  const next: Theme = effective === "dark" ? "light" : "dark";

  return (
    <button
      type="button"
      className="pmc-icon-button"
      aria-label={next === "dark" ? t("shell.theme.switchToDark") : t("shell.theme.switchToLight")}
      title={next === "dark" ? t("shell.theme.dark") : t("shell.theme.light")}
      onClick={() => {
        setTheme(next);
      }}
    >
      <svg className="pmc-nav-glyph" viewBox="0 0 18 18" aria-hidden="true" focusable="false">
        <circle cx="9" cy="9" r="6.5" />
        <path d="M9 2.5 A6.5 6.5 0 0 1 9 15.5 Z" className="pmc-theme-half" />
      </svg>
    </button>
  );
}
