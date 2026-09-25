import { useT } from "../i18n/useT";

export interface SampleBadgeProps {
  /** Opens Settings → Workspace (§4): the top-bar badge. Absent on the
   * upgrade gate, where there is no shell to navigate; it is then a label. */
  readonly onOpen?: (() => void) | undefined;
}

/**
 * The sample workspace's marker (the accepted sample-workspace amendment §5):
 * words, not colour alone. App renders it in the top bar on every route and
 * on the upgrade gate; the stylesheet lifts the top-bar one above a dialog's
 * backdrop. The window title ends in "— Sample data" as well; the host sets
 * that.
 */
export function SampleBadge({ onOpen }: SampleBadgeProps) {
  const t = useT();
  if (onOpen === undefined) {
    return (
      <p className="pmc-sample-badge pmc-sample-badge-standalone" role="note">
        {t("sampleWorkspace.badge")}
      </p>
    );
  }
  return (
    <button
      type="button"
      className="pmc-sample-badge"
      title={t("sampleWorkspace.badge.open")}
      onClick={onOpen}
    >
      {t("sampleWorkspace.badge")}
    </button>
  );
}
