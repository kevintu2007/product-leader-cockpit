import { invoke } from "@tauri-apps/api/core";

/**
 * The reviewed Evidence-from-a-file commands (item ⑦-3; DG3 Vault-root and
 * Evidence-from-file amendment §4). The picked file stays in the host behind
 * an opaque, expiring token; the webview learns its own name, when it was
 * observed, and which Evidence ids already refer to it (ADR 0011). No path
 * crosses in either direction.
 */

export interface EvidenceMatchDto {
  readonly evidenceId: string;
  readonly version: number;
}

export interface ChosenEvidenceFileDto {
  /** False when the person cancelled the picker. */
  readonly chosen: boolean;
  readonly token: string | null;
  /** The file's own name only. */
  readonly fileName: string | null;
  readonly observedAtMillis: number | null;
  /** A reference that already names this file: offered instead (§4.5). */
  readonly existing: EvidenceMatchDto | null;
  /** References holding the same content under another path: a warning. */
  readonly sameContent: readonly EvidenceMatchDto[];
}

export interface EvidenceFromFileResultDto {
  /** `file_changed`: the bytes changed since the file was chosen; the same
   * token now holds the new observation for the person to confirm. */
  readonly outcome: "created" | "file_changed" | "already_referenced";
  /** The created reference, or the one that already names the file. */
  readonly evidence: EvidenceMatchDto | null;
  /** The observation the reference records, or the new one to confirm. */
  readonly observedAtMillis: number | null;
}

/** The classifications the sheet offers: never Unclassified (§4.4). */
export type EvidenceFileClassification = "public" | "internal" | "confidential" | "restricted";

export interface EvidenceFileActions {
  /** `title` is the file dialog's title in the person's language. */
  readonly chooseEvidenceFile: (title: string) => Promise<ChosenEvidenceFileDto>;
  readonly createEvidenceFromFile: (
    token: string,
    classification: EvidenceFileClassification,
    clientRequestId: string,
  ) => Promise<EvidenceFromFileResultDto>;
  /** The sheet closed: the host forgets the chosen file. */
  readonly discardEvidenceFileChoice: () => Promise<void>;
}

export const tauriEvidenceFileActions: EvidenceFileActions = {
  chooseEvidenceFile: (title) => invoke<ChosenEvidenceFileDto>("choose_evidence_file", { title }),
  createEvidenceFromFile: (token, classification, clientRequestId) =>
    invoke<EvidenceFromFileResultDto>("create_evidence_from_file", {
      token,
      classification,
      clientRequestId,
    }),
  discardEvidenceFileChoice: () =>
    invoke<null>("discard_evidence_file_choice").then(() => undefined),
};
