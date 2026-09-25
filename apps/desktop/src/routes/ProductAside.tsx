import { useEffect, useId } from "react";

import type { LensPointDto, ProductDetailDto } from "./cockpitContract";
import type { EntryActions } from "../entry/entryIpc";
import { LensMeasures } from "./ExecutiveLens";
import { ProductInspector, type EvidenceActions } from "./ProductInspector";
import { useT } from "../i18n/useT";

/**
 * The persistent inspector beside a Product list, as DG1 draws it for the
 * Cockpit and Portfolio: the selected Product's Lens measures, then the O01
 * inspector for the same Product. Both routes use this one component so a
 * Product reads the same wherever it is opened.
 */
export interface ProductAsideProps {
  readonly ledgerRevision: number;
  readonly point: LensPointDto | null;
  /** Increases on every selection; focus moves to the measures heading when
   * it does, so a keyboard or screen reader user lands on the answer. */
  readonly focusKey: number;
  /** What to do when nothing is selected, in the route's own terms. */
  readonly emptyText: string;
  /** When absent, only the Lens measures are shown. */
  readonly loadDetail?: (productId: string) => Promise<ProductDetailDto>;
  readonly evidenceActions?: EvidenceActions;
  /** The record-entry sheets the inspector offers (slice 6B); absent means
   * the inspector reads only. */
  readonly entryActions?: EntryActions;
  /** The workspace's configured zone for the inspector's date fields. */
  readonly timeZone?: string;
  /** Re-reads the route after an inspector write, keeping the selection. */
  readonly onLedgerChanged?: () => void;
}

export function ProductAside({
  ledgerRevision,
  point,
  focusKey,
  emptyText,
  loadDetail,
  evidenceActions,
  entryActions,
  timeZone,
  onLedgerChanged,
}: ProductAsideProps) {
  const t = useT();
  const headingId = useId();

  useEffect(() => {
    if (focusKey > 0) {
      document.getElementById(headingId)?.focus();
    }
  }, [focusKey, headingId]);

  return (
    <aside className="pmc-lens-inspector" aria-label={t("productAside.label")}>
      {point === null ? (
        <p className="pmc-lens-inspector-empty">{emptyText}</p>
      ) : (
        <>
          <LensMeasures ledgerRevision={ledgerRevision} point={point} headingId={headingId} />
          {loadDetail === undefined ? null : (
            <ProductInspector
              key={point.productId}
              productId={point.productId}
              load={loadDetail}
              {...(evidenceActions === undefined ? {} : { actions: evidenceActions })}
              {...(entryActions === undefined ? {} : { entryActions })}
              {...(timeZone === undefined ? {} : { timeZone })}
              {...(onLedgerChanged === undefined ? {} : { onLedgerChanged })}
            />
          )}
        </>
      )}
    </aside>
  );
}
