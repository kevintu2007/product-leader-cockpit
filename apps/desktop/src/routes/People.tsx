import { useCallback, useEffect, useState } from "react";

import type { Translator } from "../i18n/messages";
import { useT } from "../i18n/useT";
import { resolveRejection, type ResolvedSafeError } from "../adapters/safeError";
import { formatReadAt } from "../i18n/time";
import { classificationName, workItemKindLabel } from "../i18n/workLabels";
import { SafeErrorDetail } from "../overlays/SafeErrorDetail";
import {
  newClientRequestId,
  type EntryActions,
  type EntryOutcomeDto,
  type StakeholderEntryDto,
} from "../entry/entryIpc";
import { kindWord, stakeholderFieldSpecs, stakeholderValues } from "../entry/entrySheets";
import { LinkSubjectSheet } from "../entry/LinkSubjectSheet";
import { RecordSheet } from "../entry/RecordSheet";
import type { PeopleDirectoryDto, PersonDto } from "./cockpitContract";
import { findRoute } from "../shell/routes";

/**
 * S09 People, as DG1 draws it: Stakeholder context, not a list of
 * user accounts.
 *
 * Three things this route is careful about, each because getting it wrong
 * would misstate accountability rather than merely look wrong:
 *
 * Every entry shows the **effective** classification, which composition folds
 * upward from everything the entry exposes. A person recorded as Internal who
 * is responsible for a Restricted Milestone is shown as Restricted, because
 * putting the person together with what they are accountable for is itself a
 * disclosure.
 *
 * Responsibilities and dependencies are listed separately. Being responsible
 * for something and depending on it are different relationships.
 *
 * Outstanding requests are requests, not commitments. An accepted request has
 * become an Action and is deliberately absent here.
 *
 * With `entryActions` (DG3 record-entry amendment, slice 6D) the route also
 * offers the sheets that create a Stakeholder, edit one, and relate one to a
 * subject it is responsible for or depends on. Without it the route reads
 * only.
 */
export interface PeopleProps {
  readonly load: (offset: number, limit: number) => Promise<PeopleDirectoryDto>;
  readonly entryActions?: EntryActions;
}

const PAGE_SIZE = 25;

type LoadState =
  | { readonly status: "loading" }
  | { readonly status: "error"; readonly error: ResolvedSafeError }
  | { readonly status: "ready"; readonly directory: PeopleDirectoryDto };

/** The one sheet open at a time, with the request id minted when it opened. */
type Sheet =
  | { readonly kind: "create"; readonly clientRequestId: string }
  | {
      readonly kind: "edit";
      readonly record: StakeholderEntryDto;
      readonly clientRequestId: string;
    }
  | {
      readonly kind: "link";
      readonly stakeholder: {
        readonly id: string;
        readonly label: string;
        readonly version: number;
        readonly classification: string;
      };
      readonly clientRequestId: string;
    };

function kindWords(t: Translator, kind: string): string {
  return kind === "person" || kind === "organization" ? t(`people.kind.${kind}`) : kind;
}

function subjects(person: PersonDto, purpose: string, t: Translator) {
  const names = person.relationships
    .filter((relationship) => relationship.purpose === purpose)
    .map((relationship) => relationship.subjectLabel);
  return names.length === 0 ? (
    <span className="pmc-people-none">{t("people.none")}</span>
  ) : (
    <ul className="pmc-people-list">
      {names.map((name, index) => (
        <li key={`${name}-${String(index)}`}>{name}</li>
      ))}
    </ul>
  );
}

export function People({ load, entryActions }: PeopleProps) {
  const t = useT();
  const [offset, setOffset] = useState(0);
  const [state, setState] = useState<LoadState>({ status: "loading" });
  const [sheet, setSheet] = useState<Sheet | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [opening, setOpening] = useState<ResolvedSafeError | null>(null);

  const fetchPage = useCallback(() => {
    load(offset, PAGE_SIZE).then(
      (directory) => {
        setState({ status: "ready", directory });
      },
      (reason: unknown) => {
        setState({ status: "error", error: resolveRejection(reason, t) });
      },
    );
  }, [load, offset, t]);

  const retry = useCallback(() => {
    setState({ status: "loading" });
    fetchPage();
  }, [fetchPage]);

  useEffect(() => {
    fetchPage();
  }, [fetchPage]);

  /** A write landed: say so, and re-read the directory from the Ledger. */
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
      fetchPage();
    },
    [fetchPage, t],
  );

  const openEdit = useCallback(
    (id: string) => {
      if (entryActions === undefined) {
        return;
      }
      setOpening(null);
      entryActions.loadEntryRecord("stakeholder", id).then(
        (record) => {
          if (record.kind === "stakeholder") {
            setSheet({ kind: "edit", record, clientRequestId: newClientRequestId() });
          }
        },
        (reason: unknown) => {
          setOpening(resolveRejection(reason, t));
        },
      );
    },
    [entryActions, t],
  );

  /**
   * The link sheet needs the Stakeholder's own classification to say what
   * the relationship will record; the directory shows the folded one, so
   * the record is read as the host holds it.
   */
  const openLink = useCallback(
    (id: string) => {
      if (entryActions === undefined) {
        return;
      }
      setOpening(null);
      entryActions.loadEntryRecord("stakeholder", id).then(
        (record) => {
          if (record.kind === "stakeholder") {
            setSheet({
              kind: "link",
              stakeholder: {
                id: record.id,
                label: record.name,
                version: record.version,
                classification: record.classification,
              },
              clientRequestId: newClientRequestId(),
            });
          }
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
        {t("route.loading", { route: findRoute("people").label })}
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
            route: findRoute("people").label,
            message: state.error.message,
          })}
          correlationId={state.error.correlationId}
          retryable={state.error.retryable}
        />
      </div>
    );
  }

  const { directory } = state;
  const shownFrom = directory.people.length === 0 ? 0 : directory.offset + 1;
  const shownTo = directory.offset + directory.people.length;

  function openSheet() {
    if (entryActions === undefined || sheet === null) {
      return null;
    }
    const actions = entryActions;
    switch (sheet.kind) {
      case "create":
        return (
          <RecordSheet
            title={t("entry.title.create.stakeholder")}
            fields={stakeholderFieldSpecs(t, true)}
            initial={{}}
            submitLabel={t("entry.create")}
            submit={(values) => {
              const { kind, ...fields } = stakeholderValues(values);
              return actions.createStakeholder(fields, kind, sheet.clientRequestId);
            }}
            onDone={(outcome) => {
              saved(outcome, "created");
            }}
            onClose={() => {
              setSheet(null);
            }}
          />
        );
      case "edit":
        return (
          <RecordSheet
            title={t("entry.title.edit.stakeholder")}
            fields={stakeholderFieldSpecs(t, false)}
            initial={{ name: sheet.record.name, classification: sheet.record.classification }}
            submitLabel={t("entry.save")}
            submit={(values) => {
              // The kind is not part of an edit; only what the sheet showed.
              const entered = stakeholderValues(values);
              return actions.updateStakeholder(
                sheet.record.id,
                sheet.record.version,
                { name: entered.name, classification: entered.classification },
                sheet.clientRequestId,
              );
            }}
            onDone={(outcome) => {
              saved(outcome, "updated");
            }}
            onClose={() => {
              setSheet(null);
            }}
            onStale={() => {
              setSheet(null);
              openEdit(sheet.record.id);
            }}
          />
        );
      case "link":
        return (
          <LinkSubjectSheet
            stakeholder={sheet.stakeholder}
            actions={actions}
            clientRequestId={sheet.clientRequestId}
            onDone={(outcome) => {
              saved(outcome, "linked");
            }}
            onClose={() => {
              setSheet(null);
            }}
            onStale={() => {
              setSheet(null);
              fetchPage();
            }}
          />
        );
    }
  }

  return (
    <div className="pmc-people">
      <header className="pmc-page-head">
        <div>
          <h2 className="pmc-page-headline">{t("people.headline")}</h2>
          <p className="pmc-page-lede">{t("people.lede")}</p>
        </div>
        <dl className="pmc-meta pmc-page-asof">
          <div>
            <dt>{t("route.ledgerRevision")}</dt>
            <dd>{String(directory.ledgerRevision)}</dd>
          </div>
          <div>
            <dt>{t("route.readAt")}</dt>
            <dd>
              <time>{formatReadAt(directory.asOfMillis)}</time>
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
      {entryActions === undefined ? null : (
        <div className="pmc-entry-actions pmc-people-actions">
          <button
            type="button"
            className="pmc-button pmc-button-primary"
            disabled={sheet !== null}
            onClick={() => {
              setSheet({ kind: "create", clientRequestId: newClientRequestId() });
            }}
          >
            {t("entry.new.stakeholder")}
          </button>
        </div>
      )}

      {directory.people.length === 0 ? (
        <p className="pmc-empty">{t("people.empty")}</p>
      ) : (
        <section className="pmc-lens-surface">
          <div className="pmc-lens-table">
            <table>
              <caption>
                {t("people.caption", { from: shownFrom, to: shownTo, total: directory.total })}
              </caption>
              <thead>
                <tr>
                  <th scope="col">{t("people.column.stakeholder")}</th>
                  <th scope="col">{t("people.column.kind")}</th>
                  <th scope="col">{t("people.column.responsible")}</th>
                  <th scope="col">{t("people.column.dependsOn")}</th>
                  <th scope="col">{t("people.column.waiting")}</th>
                  <th scope="col">{t("people.column.classification")}</th>
                  {entryActions === undefined ? null : (
                    <th scope="col">{t("people.column.actions")}</th>
                  )}
                </tr>
              </thead>
              <tbody>
                {directory.people.map((person) => (
                  <tr key={person.id}>
                    <th scope="row">{person.displayName}</th>
                    <td>{kindWords(t, person.kind)}</td>
                    <td>{subjects(person, "responsibility", t)}</td>
                    <td>{subjects(person, "dependency", t)}</td>
                    <td>
                      {person.outstandingRequests.length === 0 ? (
                        <span className="pmc-people-none">{t("people.none")}</span>
                      ) : (
                        <ul className="pmc-people-list">
                          {person.outstandingRequests.map((request) => (
                            <li key={request.id}>
                              <span className="pmc-kind" data-kind="action_request">
                                {workItemKindLabel(t, "action_request")}
                              </span>{" "}
                              {request.label}
                            </li>
                          ))}
                        </ul>
                      )}
                    </td>
                    <td>
                      {/* The effective classification, folded upward from
                          everything this entry exposes. */}
                      <span
                        className="pmc-classification-badge"
                        data-classification={person.classification}
                      >
                        {classificationName(t, person.classification)}
                      </span>
                    </td>
                    {entryActions === undefined ? null : (
                      <td>
                        <div className="pmc-entry-actions">
                          <button
                            type="button"
                            className="pmc-button"
                            disabled={sheet !== null}
                            onClick={() => {
                              openEdit(person.id);
                            }}
                          >
                            {t("entry.edit")}
                          </button>
                          <button
                            type="button"
                            className="pmc-button"
                            disabled={sheet !== null}
                            onClick={() => {
                              openLink(person.id);
                            }}
                          >
                            {t("entry.link.subject")}
                          </button>
                        </div>
                      </td>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="pmc-lens-paging">
            <button
              type="button"
              className="pmc-button"
              disabled={directory.offset === 0}
              onClick={() => {
                setOffset(Math.max(0, directory.offset - PAGE_SIZE));
              }}
            >
              {t("people.previous")}
            </button>
            <button
              type="button"
              className="pmc-button"
              disabled={!directory.hasMore}
              onClick={() => {
                setOffset(directory.offset + PAGE_SIZE);
              }}
            >
              {t("people.next")}
            </button>
          </div>
        </section>
      )}

      {openSheet()}
    </div>
  );
}
