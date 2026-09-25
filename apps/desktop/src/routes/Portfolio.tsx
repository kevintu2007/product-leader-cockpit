import { useCallback, useEffect, useState } from "react";

import { useT } from "../i18n/useT";
import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { formatReadAt } from "../i18n/time";
import { classificationName } from "../i18n/workLabels";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import {
  newClientRequestId,
  type EntryActions,
  type EntryOutcomeDto,
  type SimpleEntryDto,
} from "../entry/entryIpc";
import {
  kindWord,
  linkCandidates,
  linkClassificationPreview,
  linkFieldSpec,
  simpleFieldSpecs,
  simpleValues,
  type LinkCandidate,
} from "../entry/entrySheets";
import { RecordSheet } from "../entry/RecordSheet";
import type { PortfolioOverviewDto, ProductDetailDto } from "./cockpitContract";
import { LensTable } from "./ExecutiveLens";
import { ProductAside } from "./ProductAside";
import type { EvidenceActions } from "./ProductInspector";
import { findRoute } from "../shell/routes";

/**
 * S02 Portfolio.
 *
 * Every Product with the same three measures the Cockpit's Lens uses, in a
 * table, with the O01 inspector beside it as DG1 draws it. The Product list
 * stays in place while a Product is inspected (DG3 user story 4).
 *
 * The rows are in name order and the table says so: Products are not ranked
 * (DG3 S01 amendment). The flagged-work column counts work the people
 * accountable for a Product carry -- never the Product's own work, which the
 * domain does not have (DG3 O01 amendment).
 *
 * Paging is a plain accessible list with an explicit statement of what is
 * shown, rather than virtualization, which is the disposition recorded for
 * the Executive Cockpit and Portfolio routes.
 *
 * With `entryActions` (DG3 record-entry amendment, slice 6B) the route also
 * lists the Portfolios and offers the sheets that create a Portfolio or a
 * Product, edit a Portfolio, and link a Product to a Portfolio. Without it
 * the route reads only.
 */
export interface PortfolioProps {
  readonly load: (offset: number, limit: number) => Promise<PortfolioOverviewDto>;
  readonly loadDetail: (productId: string) => Promise<ProductDetailDto>;
  /** Passed straight through to the O01 inspector, which owns the Evidence
   * tab. This route neither calls them nor interprets their results. */
  readonly evidenceActions?: EvidenceActions;
  readonly entryActions?: EntryActions;
  /** The workspace's configured zone for the inspector's date fields. */
  readonly timeZone?: string;
}

type LoadState =
  | { readonly status: "loading" }
  | { readonly status: "error"; readonly error: ResolvedSafeError }
  | { readonly status: "ready"; readonly overview: PortfolioOverviewDto };

type PortfoliosState =
  | { readonly status: "loading" }
  | { readonly status: "error"; readonly error: ResolvedSafeError }
  | { readonly status: "ready"; readonly portfolios: readonly SimpleEntryDto[] };

/** The one sheet open at a time, with the request id minted when it opened. */
type Sheet =
  | { readonly kind: "createPortfolio"; readonly clientRequestId: string }
  | {
      readonly kind: "editPortfolio";
      readonly record: SimpleEntryDto;
      readonly clientRequestId: string;
    }
  | { readonly kind: "createProduct"; readonly clientRequestId: string }
  | {
      readonly kind: "linkProduct";
      readonly portfolio: SimpleEntryDto;
      readonly candidates: readonly LinkCandidate[];
      readonly clientRequestId: string;
    };

const PAGE_SIZE = 25;

export function Portfolio({
  load,
  loadDetail,
  evidenceActions,
  entryActions,
  timeZone,
}: PortfolioProps) {
  const t = useT();
  const [state, setState] = useState<LoadState>({ status: "loading" });
  const [offset, setOffset] = useState(0);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [focusKey, setFocusKey] = useState(0);
  const [portfolios, setPortfolios] = useState<PortfoliosState>({ status: "loading" });
  const [sheet, setSheet] = useState<Sheet | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [opening, setOpening] = useState<ResolvedSafeError | null>(null);

  const fetchPage = useCallback(() => {
    load(offset, PAGE_SIZE).then(
      (overview) => {
        setState({ status: "ready", overview });
      },
      (reason: unknown) => {
        setState({ status: "error", error: resolveRejection(reason, t) });
      },
    );
  }, [load, offset, t]);

  const fetchPortfolios = useCallback(() => {
    if (entryActions === undefined) {
      return;
    }
    entryActions.listEntryRecords("portfolio").then(
      (list) => {
        setPortfolios({
          status: "ready",
          portfolios: list.records.filter(
            (record): record is SimpleEntryDto & { readonly kind: "portfolio" } =>
              record.kind === "portfolio",
          ),
        });
      },
      (reason: unknown) => {
        setPortfolios({ status: "error", error: resolveRejection(reason, t) });
      },
    );
  }, [entryActions, t]);

  const retry = useCallback(() => {
    setState({ status: "loading" });
    fetchPage();
  }, [fetchPage]);

  useEffect(() => {
    fetchPage();
  }, [fetchPage]);

  useEffect(() => {
    fetchPortfolios();
  }, [fetchPortfolios]);

  const select = useCallback((productId: string) => {
    setSelectedId(productId);
    setFocusKey((key) => key + 1);
  }, []);

  /** A write landed: say so, and re-read both lists from the Ledger. */
  const saved = useCallback(
    (outcome: EntryOutcomeDto, how: "created" | "updated" | "linked") => {
      setSheet(null);
      setNotice(
        how === "linked"
          ? t("entry.saved.linked", { id: outcome.id })
          : t(how === "created" ? "entry.saved.created" : "entry.saved.updated", {
              kind: kindWord(t, outcome.kind),
              id: outcome.id,
            }),
      );
      fetchPortfolios();
      fetchPage();
    },
    [fetchPage, fetchPortfolios, t],
  );

  const openEditPortfolio = useCallback(
    (id: string) => {
      if (entryActions === undefined) {
        return;
      }
      setOpening(null);
      entryActions.loadEntryRecord("portfolio", id).then(
        (record) => {
          if (record.kind === "portfolio") {
            setSheet({ kind: "editPortfolio", record, clientRequestId: newClientRequestId() });
          }
        },
        (reason: unknown) => {
          setOpening(resolveRejection(reason, t));
        },
      );
    },
    [entryActions, t],
  );

  const openLinkProduct = useCallback(
    (portfolio: SimpleEntryDto) => {
      if (entryActions === undefined) {
        return;
      }
      setOpening(null);
      entryActions.listEntryRecords("product").then(
        (list) => {
          setSheet({
            kind: "linkProduct",
            portfolio,
            candidates: linkCandidates(
              list.records.filter((record) => record.kind === "product"),
              new Set(),
            ),
            clientRequestId: newClientRequestId(),
          });
        },
        (reason: unknown) => {
          setOpening(resolveRejection(reason, t));
        },
      );
    },
    [entryActions, t],
  );

  if (state.status === "loading") {
    return (
      <p className="pmc-cockpit-status" role="status">
        {t("route.loading", { route: findRoute("portfolio").label })}
      </p>
    );
  }

  if (state.status === "error") {
    return (
      <div className="pmc-cockpit-status">
        <button type="button" className="pmc-button" onClick={retry}>
          {t("route.reload")}
        </button>
        <SafeErrorDetail
          message={t("route.unavailable", {
            route: findRoute("portfolio").label,
            message: state.error.message,
          })}
          correlationId={state.error.correlationId}
          retryable={state.error.retryable}
        />
      </div>
    );
  }

  const { overview } = state;

  if (overview.state === "outOfSync") {
    return (
      <div className="pmc-cockpit-status">
        <p role="status">{t("route.outOfSync")}</p>
        <button type="button" className="pmc-button" onClick={retry}>
          {t("route.reload")}
        </button>
      </div>
    );
  }

  const points = overview.rows.map((row) => row.point);
  const rowFor = (productId: string) =>
    overview.rows.find((row) => row.point.productId === productId);
  const selected = points.find((point) => point.productId === selectedId) ?? null;
  const shownFrom = overview.offset + 1;
  const shownTo = overview.offset + overview.rows.length;

  function openSheet() {
    if (entryActions === undefined || sheet === null) {
      return null;
    }
    const actions = entryActions;
    switch (sheet.kind) {
      case "createPortfolio":
        return (
          <RecordSheet
            title={t("entry.title.create.portfolio")}
            fields={simpleFieldSpecs(t)}
            initial={{}}
            submitLabel={t("entry.create")}
            submit={(values) =>
              actions.createPortfolio(simpleValues(values), sheet.clientRequestId)
            }
            onDone={(outcome) => {
              saved(outcome, "created");
            }}
            onClose={() => {
              setSheet(null);
            }}
          />
        );
      case "editPortfolio":
        return (
          <RecordSheet
            title={t("entry.title.edit.portfolio")}
            fields={simpleFieldSpecs(t)}
            initial={{
              name: sheet.record.name,
              details: sheet.record.details,
              classification: sheet.record.classification,
            }}
            submitLabel={t("entry.save")}
            submit={(values) =>
              actions.updatePortfolio(
                sheet.record.id,
                sheet.record.version,
                simpleValues(values),
                sheet.clientRequestId,
              )
            }
            onDone={(outcome) => {
              saved(outcome, "updated");
            }}
            onClose={() => {
              setSheet(null);
            }}
            onStale={() => {
              setSheet(null);
              openEditPortfolio(sheet.record.id);
            }}
          />
        );
      case "createProduct":
        return (
          <RecordSheet
            title={t("entry.title.create.product")}
            fields={simpleFieldSpecs(t)}
            initial={{}}
            submitLabel={t("entry.create")}
            submit={(values) => actions.createProduct(simpleValues(values), sheet.clientRequestId)}
            onDone={(outcome) => {
              saved(outcome, "created");
            }}
            onClose={() => {
              setSheet(null);
            }}
          />
        );
      case "linkProduct":
        return (
          <RecordSheet
            title={t("entry.title.link.product", { portfolio: sheet.portfolio.name })}
            fields={[linkFieldSpec(t, sheet.candidates)]}
            initial={{}}
            submitLabel={t("entry.linkConfirm")}
            preview={(values) =>
              linkClassificationPreview(t, sheet.portfolio.classification, sheet.candidates, values)
            }
            submit={(values) => {
              const chosen = sheet.candidates.find((candidate) => candidate.id === values.record);
              if (chosen === undefined) {
                return Promise.reject(new Error("no candidate chosen"));
              }
              return actions.linkPortfolioProduct(
                sheet.portfolio.id,
                sheet.portfolio.version,
                chosen.id,
                chosen.version,
                sheet.clientRequestId,
              );
            }}
            onDone={(outcome) => {
              saved(outcome, "linked");
            }}
            onClose={() => {
              setSheet(null);
            }}
            onStale={() => {
              setSheet(null);
              fetchPortfolios();
            }}
          />
        );
    }
  }

  function portfoliosSection() {
    if (entryActions === undefined) {
      return null;
    }
    return (
      <section
        className="pmc-lens-surface pmc-entry-portfolios"
        aria-labelledby="pmc-portfolio-portfolios"
      >
        <div className="pmc-lens-head">
          <div>
            <h3 id="pmc-portfolio-portfolios">{t("entry.portfolios")}</h3>
          </div>
          <div className="pmc-entry-actions">
            <button
              type="button"
              className="pmc-button pmc-button-primary"
              disabled={sheet !== null}
              onClick={() => {
                setSheet({ kind: "createPortfolio", clientRequestId: newClientRequestId() });
              }}
            >
              {t("entry.new.portfolio")}
            </button>
          </div>
        </div>
        {portfolios.status === "loading" ? (
          <p className="pmc-lens-empty" role="status">
            {t("entry.portfolios.loading")}
          </p>
        ) : portfolios.status === "error" ? (
          <div className="pmc-lens-empty">
            <SafeErrorDetail
              message={t("entry.portfolios.unavailable", { message: portfolios.error.message })}
              correlationId={portfolios.error.correlationId}
              retryable={portfolios.error.retryable}
            />
          </div>
        ) : portfolios.portfolios.length === 0 ? (
          <p className="pmc-lens-empty">{t("entry.portfolios.none")}</p>
        ) : (
          <ul className="pmc-entry-list">
            {portfolios.portfolios.map((portfolio) => (
              <li key={portfolio.id}>
                <span>
                  {t("entry.portfolio.line", {
                    name: portfolio.name,
                    classification: classificationName(t, portfolio.classification),
                    version: String(portfolio.version),
                  })}
                </span>
                <div className="pmc-entry-actions">
                  <button
                    type="button"
                    className="pmc-button"
                    disabled={sheet !== null}
                    onClick={() => {
                      openEditPortfolio(portfolio.id);
                    }}
                  >
                    {t("entry.edit")}
                  </button>
                  <button
                    type="button"
                    className="pmc-button"
                    disabled={sheet !== null}
                    onClick={() => {
                      openLinkProduct(portfolio);
                    }}
                  >
                    {t("entry.link.product")}
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </section>
    );
  }

  return (
    <div className="pmc-portfolio">
      <header className="pmc-page-head">
        <div>
          <h2 className="pmc-page-headline">{t("portfolio.headline")}</h2>
          <p className="pmc-page-lede">{t("portfolio.lede")}</p>
        </div>
        <dl className="pmc-meta pmc-page-asof">
          <div>
            <dt>{t("route.ledgerRevision")}</dt>
            <dd>{String(overview.ledgerRevision)}</dd>
          </div>
          <div>
            <dt>{t("route.readAt")}</dt>
            <dd>
              <time>{formatReadAt(overview.asOfMillis)}</time>
            </dd>
          </div>
        </dl>
      </header>

      {notice !== null ? (
        <p className="pmc-entry-notice" role="status">
          {notice}
        </p>
      ) : null}
      {opening !== null ? (
        <SafeErrorDetail
          message={t("entry.readFailed", { message: opening.message })}
          correlationId={opening.correlationId}
          retryable={opening.retryable}
        />
      ) : null}

      {portfoliosSection()}

      <div className="pmc-lens-layout">
        <section className="pmc-lens-surface" aria-labelledby="pmc-portfolio-products">
          <div className="pmc-lens-head">
            <div>
              <h3 id="pmc-portfolio-products">{t("noun.products")}</h3>
              {overview.rows.length === 0 ? null : (
                <p>
                  {t("portfolio.showing", {
                    total: overview.total,
                    from: shownFrom,
                    to: shownTo,
                  })}
                </p>
              )}
            </div>
            {entryActions === undefined ? null : (
              <div className="pmc-entry-actions">
                <button
                  type="button"
                  className="pmc-button pmc-button-primary"
                  disabled={sheet !== null}
                  onClick={() => {
                    setSheet({ kind: "createProduct", clientRequestId: newClientRequestId() });
                  }}
                >
                  {t("entry.new.product")}
                </button>
              </div>
            )}
          </div>
          {overview.total === 0 ? (
            <p className="pmc-lens-empty">{t("portfolio.empty")}</p>
          ) : overview.rows.length === 0 ? (
            // Products exist, but not on this page (some were removed since
            // the page was chosen). Saying the Ledger is empty would be false.
            <div className="pmc-lens-empty">
              <p>{t("portfolio.emptyPage", { total: overview.total })}</p>
              <button
                type="button"
                className="pmc-button"
                onClick={() => {
                  setOffset(0);
                }}
              >
                {t("portfolio.firstPage")}
              </button>
            </div>
          ) : (
            <LensTable
              points={points}
              selectedId={selectedId}
              onSelect={select}
              extraColumns={[
                {
                  header: t("portfolio.column.flagged"),
                  cell: (point) => {
                    const count = rowFor(point.productId)?.flaggedWorkCount ?? 0;
                    return count === 0
                      ? t("portfolio.flaggedNone")
                      : t("portfolio.flaggedCount", { count });
                  },
                },
                {
                  header: t("portfolio.column.classification"),
                  cell: (point) => {
                    const classification =
                      rowFor(point.productId)?.classification ?? point.effectiveClassification;
                    return (
                      <span
                        className="pmc-classification-badge"
                        data-classification={classification}
                      >
                        {classificationName(t, classification)}
                      </span>
                    );
                  },
                },
              ]}
            />
          )}
          {overview.hasMore ? (
            <div className="pmc-lens-paging">
              <button
                type="button"
                className="pmc-button"
                onClick={() => {
                  setOffset(overview.offset + PAGE_SIZE);
                }}
              >
                {t("portfolio.nextPage")}
              </button>
            </div>
          ) : null}
        </section>

        <ProductAside
          ledgerRevision={overview.ledgerRevision}
          point={selected}
          focusKey={focusKey}
          emptyText={t("portfolio.asideEmpty")}
          loadDetail={loadDetail}
          {...(evidenceActions === undefined ? {} : { evidenceActions })}
          {...(entryActions === undefined ? {} : { entryActions })}
          {...(timeZone === undefined ? {} : { timeZone })}
          onLedgerChanged={fetchPage}
        />
      </div>

      {openSheet()}
    </div>
  );
}
