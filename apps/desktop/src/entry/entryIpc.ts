import { invoke } from "@tauri-apps/api/core";

/**
 * Record entry for the Portfolio family (slice 6B; DG3 record-entry
 * amendment §2–§4). The webview sends the fields the person entered, the id
 * and version of a record it read, and one `clientRequestId` per opened
 * sheet; the host mints every new id, the correlation, the audit ids and
 * the instant, and fixes provenance to "entered by a person". No path
 * crosses in either direction. Mirrors `entry_commands.rs`.
 */

export type ListableKind =
  "portfolio" | "product" | "roadmap" | "kpi_definition" | "initiative" | "project" | "stakeholder";

export type EntryKind = ListableKind | "kpi_observation" | "milestone";

/** A Portfolio, Product or Roadmap as a sheet edits it. */
export interface SimpleEntryDto {
  readonly id: string;
  readonly name: string;
  readonly details: string;
  readonly classification: string;
  readonly version: number;
}

export interface KpiDefinitionEntryDto {
  readonly id: string;
  readonly name: string;
  readonly definition: string;
  readonly owner: string;
  readonly target: string;
  readonly cadence: string;
  readonly source: string;
  readonly classification: string;
  readonly version: number;
}

export interface KpiObservationEntryDto {
  readonly id: string;
  readonly kpiId: string;
  readonly value: string;
  readonly observedAtMillis: number;
  readonly source: string;
  readonly classification: string;
  readonly version: number;
}

export interface InitiativeEntryDto {
  readonly id: string;
  readonly name: string;
  readonly definedOutcome: string;
  readonly classification: string;
  readonly version: number;
}

export interface ProjectEntryDto {
  readonly id: string;
  readonly name: string;
  readonly startAtMillis: number;
  readonly endAtMillis: number;
  readonly classification: string;
  readonly version: number;
}

export interface MilestoneEntryDto {
  readonly id: string;
  readonly projectId: string;
  readonly name: string;
  readonly verificationCriteria: string;
  readonly dueAtMillis: number;
  readonly classification: string;
  readonly version: number;
}

export interface StakeholderEntryDto {
  readonly id: string;
  readonly name: string;
  /** `person` or `organization`; not `kind`, which is the record's tag. */
  readonly stakeholderKind: string;
  readonly classification: string;
  readonly version: number;
}

export type EntryRecordDto =
  | ({ readonly kind: "portfolio" } & SimpleEntryDto)
  | ({ readonly kind: "product" } & SimpleEntryDto)
  | ({ readonly kind: "roadmap" } & SimpleEntryDto)
  | ({ readonly kind: "kpi_definition" } & KpiDefinitionEntryDto)
  | ({ readonly kind: "kpi_observation" } & KpiObservationEntryDto)
  | ({ readonly kind: "initiative" } & InitiativeEntryDto)
  | ({ readonly kind: "project" } & ProjectEntryDto)
  | ({ readonly kind: "milestone" } & MilestoneEntryDto)
  | ({ readonly kind: "stakeholder" } & StakeholderEntryDto);

export interface EntryListDto {
  readonly ledgerRevision: number;
  readonly records: readonly EntryRecordDto[];
}

/** What every create, edit and link answers with: the record as the Ledger
 * now holds it, and the correlation of this command. */
export interface EntryOutcomeDto {
  readonly kind: string;
  readonly id: string;
  readonly classification: string;
  readonly version: number;
  readonly correlationId: string;
}

/** `classification` is `null` only on an edit that keeps the current one, or
 * an observation that takes its KPI's. A create chooses (§3.3). */
export interface SimpleFields {
  readonly name: string;
  readonly details: string;
  readonly classification: string | null;
}

export interface KpiDefinitionFields {
  readonly name: string;
  readonly definition: string;
  readonly owner: string;
  readonly target: string;
  readonly cadence: string;
  readonly source: string;
  readonly classification: string | null;
}

export interface KpiObservationFields {
  readonly value: string;
  readonly observedAtMillis: number;
  readonly source: string;
  readonly classification: string | null;
}

export interface InitiativeFields {
  readonly name: string;
  readonly definedOutcome: string;
  readonly classification: string | null;
}

export interface ProjectFields {
  readonly name: string;
  readonly startAtMillis: number;
  readonly endAtMillis: number;
  readonly classification: string | null;
}

export interface MilestoneFields {
  readonly name: string;
  readonly verificationCriteria: string;
  readonly dueAtMillis: number;
  readonly classification: string | null;
}

export interface StakeholderFields {
  readonly name: string;
  readonly classification: string | null;
}

// Work (slice 6E). Optional fields are `null` when the person left them
// empty; the host refuses a blank text it would otherwise store.
export interface ActionRequestFields {
  readonly title: string;
  readonly details: string;
  readonly intendedOwnerId: string | null;
  readonly responseDueAtMillis: number | null;
  readonly intendedActionDueAtMillis: number | null;
  readonly classification: string | null;
}

export interface DecisionRequestFields {
  readonly subject: string;
  readonly details: string;
  readonly intendedOwnerId: string | null;
  readonly classification: string | null;
}

export interface IssueFields {
  readonly title: string;
  readonly details: string;
  readonly classification: string | null;
  readonly recurrenceOfId: string | null;
}

export interface RiskFields {
  readonly title: string;
  readonly details: string;
  readonly classification: string | null;
}

/** `mitigate`, `accept`, `transfer` or `avoid`; accept and transfer need
 * every other field, which the host enforces. */
export interface RiskResponseFields {
  readonly response: string;
  readonly ownerId: string | null;
  readonly rationale: string | null;
  readonly residualExposure: string | null;
  readonly nextReviewAtMillis: number | null;
}

export interface EntryActions {
  readonly listEntryRecords: (kind: ListableKind) => Promise<EntryListDto>;
  readonly loadEntryRecord: (kind: EntryKind, id: string) => Promise<EntryRecordDto>;
  readonly createPortfolio: (
    fields: SimpleFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly updatePortfolio: (
    id: string,
    expectedVersion: number,
    fields: SimpleFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly createProduct: (
    fields: SimpleFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly updateProduct: (
    id: string,
    expectedVersion: number,
    fields: SimpleFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly createRoadmap: (
    fields: SimpleFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly updateRoadmap: (
    id: string,
    expectedVersion: number,
    fields: SimpleFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly createKpiDefinition: (
    fields: KpiDefinitionFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly updateKpiDefinition: (
    id: string,
    expectedVersion: number,
    fields: KpiDefinitionFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  /** An observation under the KPI the sheet read, at the version it read. */
  readonly createKpiObservation: (
    kpiId: string,
    kpiVersion: number,
    fields: KpiObservationFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly updateKpiObservation: (
    id: string,
    expectedVersion: number,
    fields: KpiObservationFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly linkPortfolioProduct: (
    portfolioId: string,
    portfolioVersion: number,
    productId: string,
    productVersion: number,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly linkProductRoadmap: (
    productId: string,
    productVersion: number,
    roadmapId: string,
    roadmapVersion: number,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly linkProductKpi: (
    productId: string,
    productVersion: number,
    kpiId: string,
    kpiVersion: number,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  // The Delivery family (slice 6C).
  readonly createInitiative: (
    fields: InitiativeFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly updateInitiative: (
    id: string,
    expectedVersion: number,
    fields: InitiativeFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly createProject: (
    fields: ProjectFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly updateProject: (
    id: string,
    expectedVersion: number,
    fields: ProjectFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  /** A Milestone is created under the Project the sheet was opened from, at
   * the Project version the sheet read. */
  readonly createMilestone: (
    projectId: string,
    projectVersion: number,
    fields: MilestoneFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly updateMilestone: (
    id: string,
    expectedVersion: number,
    fields: MilestoneFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly linkInitiativeProject: (
    initiativeId: string,
    initiativeVersion: number,
    projectId: string,
    projectVersion: number,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly linkProjectProduct: (
    projectId: string,
    projectVersion: number,
    productId: string,
    productVersion: number,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  // People (slice 6D).
  readonly createStakeholder: (
    fields: StakeholderFields,
    kind: string,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly updateStakeholder: (
    id: string,
    expectedVersion: number,
    fields: StakeholderFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  /** A Stakeholder's relationship to a subject it read, at both versions. */
  readonly linkStakeholderSubject: (
    stakeholderId: string,
    stakeholderVersion: number,
    subjectKind: string,
    subjectId: string,
    subjectVersion: number,
    purpose: string,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  // Work (slice 6E).
  readonly createActionRequestDraft: (
    fields: ActionRequestFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  /** Draft -> Open, at the version the row read. */
  readonly submitActionRequest: (
    id: string,
    expectedVersion: number,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly createDecisionRequestDraft: (
    fields: DecisionRequestFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly submitDecisionRequest: (
    id: string,
    expectedVersion: number,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
  readonly createIssue: (fields: IssueFields, clientRequestId: string) => Promise<EntryOutcomeDto>;
  readonly createRisk: (fields: RiskFields, clientRequestId: string) => Promise<EntryOutcomeDto>;
  readonly updateRiskResponse: (
    id: string,
    expectedVersion: number,
    fields: RiskResponseFields,
    clientRequestId: string,
  ) => Promise<EntryOutcomeDto>;
}

export const tauriEntryActions: EntryActions = {
  listEntryRecords: (kind) => invoke<EntryListDto>("list_entry_records", { kind }),
  loadEntryRecord: (kind, id) => invoke<EntryRecordDto>("get_entry_record", { kind, id }),
  createPortfolio: (fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("create_portfolio_record", { ...fields, clientRequestId }),
  updatePortfolio: (id, expectedVersion, fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("update_portfolio_record", {
      id,
      expectedVersion,
      ...fields,
      clientRequestId,
    }),
  createProduct: (fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("create_product_record", { ...fields, clientRequestId }),
  updateProduct: (id, expectedVersion, fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("update_product_record", {
      id,
      expectedVersion,
      ...fields,
      clientRequestId,
    }),
  createRoadmap: (fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("create_roadmap_record", { ...fields, clientRequestId }),
  updateRoadmap: (id, expectedVersion, fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("update_roadmap_record", {
      id,
      expectedVersion,
      ...fields,
      clientRequestId,
    }),
  createKpiDefinition: (fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("create_kpi_definition_record", { ...fields, clientRequestId }),
  updateKpiDefinition: (id, expectedVersion, fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("update_kpi_definition_record", {
      id,
      expectedVersion,
      ...fields,
      clientRequestId,
    }),
  createKpiObservation: (kpiId, kpiVersion, fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("create_kpi_observation_record", {
      kpiId,
      kpiVersion,
      ...fields,
      clientRequestId,
    }),
  updateKpiObservation: (id, expectedVersion, fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("update_kpi_observation_record", {
      id,
      expectedVersion,
      ...fields,
      clientRequestId,
    }),
  linkPortfolioProduct: (
    portfolioId,
    portfolioVersion,
    productId,
    productVersion,
    clientRequestId,
  ) =>
    invoke<EntryOutcomeDto>("link_portfolio_product_record", {
      portfolioId,
      portfolioVersion,
      productId,
      productVersion,
      clientRequestId,
    }),
  linkProductRoadmap: (productId, productVersion, roadmapId, roadmapVersion, clientRequestId) =>
    invoke<EntryOutcomeDto>("link_product_roadmap_record", {
      productId,
      productVersion,
      roadmapId,
      roadmapVersion,
      clientRequestId,
    }),
  linkProductKpi: (productId, productVersion, kpiId, kpiVersion, clientRequestId) =>
    invoke<EntryOutcomeDto>("link_product_kpi_record", {
      productId,
      productVersion,
      kpiId,
      kpiVersion,
      clientRequestId,
    }),
  createInitiative: (fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("create_initiative_record", { ...fields, clientRequestId }),
  updateInitiative: (id, expectedVersion, fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("update_initiative_record", {
      id,
      expectedVersion,
      ...fields,
      clientRequestId,
    }),
  createProject: (fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("create_project_record", { ...fields, clientRequestId }),
  updateProject: (id, expectedVersion, fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("update_project_record", {
      id,
      expectedVersion,
      ...fields,
      clientRequestId,
    }),
  createMilestone: (projectId, projectVersion, fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("create_milestone_record", {
      projectId,
      projectVersion,
      ...fields,
      clientRequestId,
    }),
  updateMilestone: (id, expectedVersion, fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("update_milestone_record", {
      id,
      expectedVersion,
      ...fields,
      clientRequestId,
    }),
  linkInitiativeProject: (
    initiativeId,
    initiativeVersion,
    projectId,
    projectVersion,
    clientRequestId,
  ) =>
    invoke<EntryOutcomeDto>("link_initiative_project_record", {
      initiativeId,
      initiativeVersion,
      projectId,
      projectVersion,
      clientRequestId,
    }),
  linkProjectProduct: (projectId, projectVersion, productId, productVersion, clientRequestId) =>
    invoke<EntryOutcomeDto>("link_project_product_record", {
      projectId,
      projectVersion,
      productId,
      productVersion,
      clientRequestId,
    }),
  createStakeholder: (fields, kind, clientRequestId) =>
    invoke<EntryOutcomeDto>("create_stakeholder_record", { ...fields, kind, clientRequestId }),
  updateStakeholder: (id, expectedVersion, fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("update_stakeholder_record", {
      id,
      expectedVersion,
      ...fields,
      clientRequestId,
    }),
  linkStakeholderSubject: (
    stakeholderId,
    stakeholderVersion,
    subjectKind,
    subjectId,
    subjectVersion,
    purpose,
    clientRequestId,
  ) =>
    invoke<EntryOutcomeDto>("link_stakeholder_subject_record", {
      stakeholderId,
      stakeholderVersion,
      subjectKind,
      subjectId,
      subjectVersion,
      purpose,
      clientRequestId,
    }),
  createActionRequestDraft: (fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("create_action_request_draft_record", { ...fields, clientRequestId }),
  submitActionRequest: (id, expectedVersion, clientRequestId) =>
    invoke<EntryOutcomeDto>("submit_action_request_record", {
      id,
      expectedVersion,
      clientRequestId,
    }),
  createDecisionRequestDraft: (fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("create_decision_request_draft_record", {
      ...fields,
      clientRequestId,
    }),
  submitDecisionRequest: (id, expectedVersion, clientRequestId) =>
    invoke<EntryOutcomeDto>("submit_decision_request_record", {
      id,
      expectedVersion,
      clientRequestId,
    }),
  createIssue: (fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("create_issue_record", { ...fields, clientRequestId }),
  createRisk: (fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("create_risk_record", { ...fields, clientRequestId }),
  updateRiskResponse: (id, expectedVersion, fields, clientRequestId) =>
    invoke<EntryOutcomeDto>("update_risk_response_record", {
      id,
      expectedVersion,
      ...fields,
      clientRequestId,
    }),
};
/**
 * The workspace's configured time zone (§3.5), an IANA name the host's
 * settings document validated. Every date a sheet takes is entered in it.
 */
export function loadDisplayTimezone(): Promise<string> {
  return invoke<string>("get_display_timezone");
}

/** One request id per opened sheet, reused verbatim on a retry of the same
 * submit, so a retry can never become a second record (§3.7). */
export function newClientRequestId(): string {
  const random =
    typeof crypto !== "undefined" && typeof crypto.randomUUID === "function"
      ? crypto.randomUUID()
      : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `entry-${random}`;
}
