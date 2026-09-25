import { vi } from "vitest";

import type { EntryActions, EntryOutcomeDto, EntryRecordDto } from "./entryIpc";

/**
 * A test double for the record-entry host commands: every list is empty,
 * every read is missing, and every write answers with the outcome the
 * command names. A test overrides what it exercises.
 */
export function outcome(kind: string, id: string, version = 1): EntryOutcomeDto {
  return { kind, id, classification: "internal", version, correlationId: `corr-${id}` };
}

export function entryTestActions(
  overrides: Partial<EntryActions> = {},
  records: readonly EntryRecordDto[] = [],
): EntryActions {
  return {
    listEntryRecords: vi.fn((kind) =>
      Promise.resolve({
        ledgerRevision: 1,
        records: records.filter((record) => record.kind === kind),
      }),
    ),
    loadEntryRecord: vi.fn((kind, id) => {
      const found = records.find((record) => record.kind === kind && record.id === id);
      return found === undefined
        ? Promise.reject(new Error("no such record"))
        : Promise.resolve(found);
    }),
    createPortfolio: vi.fn(() => Promise.resolve(outcome("portfolio", "portfolio-new"))),
    updatePortfolio: vi.fn((id: string) => Promise.resolve(outcome("portfolio", id, 2))),
    createProduct: vi.fn(() => Promise.resolve(outcome("product", "product-new"))),
    updateProduct: vi.fn((id: string) => Promise.resolve(outcome("product", id, 2))),
    createRoadmap: vi.fn(() => Promise.resolve(outcome("roadmap", "roadmap-new"))),
    updateRoadmap: vi.fn((id: string) => Promise.resolve(outcome("roadmap", id, 2))),
    createKpiDefinition: vi.fn(() => Promise.resolve(outcome("kpi_definition", "kpi-new"))),
    updateKpiDefinition: vi.fn((id: string) => Promise.resolve(outcome("kpi_definition", id, 2))),
    createKpiObservation: vi.fn(() =>
      Promise.resolve(outcome("kpi_observation", "observation-new")),
    ),
    updateKpiObservation: vi.fn((id: string) => Promise.resolve(outcome("kpi_observation", id, 2))),
    linkPortfolioProduct: vi.fn(() => Promise.resolve(outcome("relationship", "relationship-1"))),
    linkProductRoadmap: vi.fn(() => Promise.resolve(outcome("relationship", "relationship-2"))),
    linkProductKpi: vi.fn(() => Promise.resolve(outcome("relationship", "relationship-3"))),
    createInitiative: vi.fn(() => Promise.resolve(outcome("initiative", "initiative-new"))),
    updateInitiative: vi.fn((id: string) => Promise.resolve(outcome("initiative", id, 2))),
    createProject: vi.fn(() => Promise.resolve(outcome("project", "project-new"))),
    updateProject: vi.fn((id: string) => Promise.resolve(outcome("project", id, 2))),
    createMilestone: vi.fn(() => Promise.resolve(outcome("milestone", "milestone-new"))),
    updateMilestone: vi.fn((id: string) => Promise.resolve(outcome("milestone", id, 2))),
    linkInitiativeProject: vi.fn(() => Promise.resolve(outcome("relationship", "relationship-4"))),
    linkProjectProduct: vi.fn(() => Promise.resolve(outcome("relationship", "relationship-5"))),
    createStakeholder: vi.fn(() => Promise.resolve(outcome("stakeholder", "stakeholder-new"))),
    updateStakeholder: vi.fn((id: string) => Promise.resolve(outcome("stakeholder", id, 2))),
    linkStakeholderSubject: vi.fn(() => Promise.resolve(outcome("relationship", "relationship-6"))),
    createActionRequestDraft: vi.fn(() =>
      Promise.resolve(outcome("action_request", "request-new")),
    ),
    submitActionRequest: vi.fn((id: string) => Promise.resolve(outcome("action_request", id, 2))),
    createDecisionRequestDraft: vi.fn(() =>
      Promise.resolve(outcome("decision_request", "decision-request-new")),
    ),
    submitDecisionRequest: vi.fn((id: string) =>
      Promise.resolve(outcome("decision_request", id, 2)),
    ),
    createIssue: vi.fn(() => Promise.resolve(outcome("issue", "issue-new"))),
    createRisk: vi.fn(() => Promise.resolve(outcome("risk", "risk-new"))),
    updateRiskResponse: vi.fn((id: string) => Promise.resolve(outcome("risk", id, 2))),
    ...overrides,
  };
}
