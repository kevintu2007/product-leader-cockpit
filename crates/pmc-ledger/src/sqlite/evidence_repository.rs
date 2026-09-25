//! Typed SQLite persistence seam for the general-purpose Evidence family
//! (Evidence/Vault): `CreateEvidenceReference` and `LinkEvidence`.
//! Distinct from `issue_repository.rs`'s narrow, read-only use of
//! `evidence_references` for completion-witness joins.
//!
//! Follows the same lightweight, direct-SQL convention proven for the
//! Portfolio family's SQLite writers: this repository reimplements H1-User
//! validation directly against SQL rather than rehydrating a domain-crate
//! in-memory service, using `pmc_domain::evidence`'s value types as
//! validation building blocks only.
//!
//! Evidence audits use `AuditModule::WorkManagement` (the module the narrow,
//! pre-existing completion-evidence concept already lives under) and the
//! fixed effect code [`EVIDENCE_EFFECT_CODE`].
//!
//! `evidence_links.target_type` uses the same long aggregate-type
//! vocabulary as `aggregate_registry.aggregate_type` directly (e.g.
//! `kpi_definition`, not the short `kpi` used by Relationship's own
//! endpoint tables) -- a plain link to an existing aggregate row has no
//! need for Relationship's short/long split. Re-linking an already-linked
//! `(evidence_id, target)` pair under a *different* idempotency ID is a
//! deliberate scope decision: unlike ordinary Relationship linking's
//! semantic-reuse no-op, this repository rejects it as `DomainConflict` --
//! simpler and safe, since Evidence links are informational, not
//! authority-defining.

use pmc_domain::{
    audit::{
        AuditAction, AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectCode,
        AuditEffectScope, AuditEvent, AuditEventCode, AuditExecutionOutcome, AuditModule,
        AuditPolicyOutcome, AuditTarget,
    },
    classification::DataClassification,
    error::{DomainError, ErrorCode, MessageKey},
    evidence::{
        execute_supersede_evidence_reference,
        prepare_supersede_evidence_reference as prepare_supersession,
        ApproveAndExecuteSupersedeEvidenceReference, CreateEvidenceReference, EvidenceFingerprint,
        EvidenceLinkRecord, EvidenceLinkTarget, EvidenceReferenceRecord, EvidenceSupersessionError,
        EvidenceSupersessionLinkSnapshot, EvidenceSupersessionPayloadDigest,
        EvidenceSupersessionPreparedIntent, EvidenceSupersessionSourceSnapshot,
        FingerprintAlgorithm, LinkEvidence, MutationOutcome, OperationContext,
        PinEvidenceFingerprint, PrepareSupersedeEvidenceReference, RelocateEvidenceReference,
        UpdateEvidenceVerification, VaultRelativePath,
    },
    identity::{
        ActionId, ActionRequestId, AggregateVersion, ApprovalReceiptId, AuditEventId,
        CorrelationId, DecisionId, DecisionRequestId, EvidenceReferenceId, IdempotencyId,
        InitiativeId, IssueId, KpiId, KpiObservationId, MilestoneId, PreparedIntentId, ProductId,
        ProjectId, RiskId, RoadmapId,
    },
    provenance::{Provenance, ProvenanceReference},
    time::UtcTimestamp,
    work_management::{EvidenceVerification, IntegrityDigest, WorkManagementRationale},
};
use rusqlite::{OptionalExtension, Transaction};

use super::reservation_repository::{
    reservation_matches, ReservationRequest, ReservedEntityKind, ReservedId,
};
use super::{LedgerOpenError, LedgerTransactionError, SqliteProductLedger, CURRENT_SCHEMA_VERSION};

/// The reservation an Evidence-from-a-file create names (schema v48).
const EVIDENCE_RESERVATION: ReservationRequest = ReservationRequest {
    kind: ReservedEntityKind::EvidenceReference,
    operation: "create_evidence_reference",
};

/// An existing Evidence reference, named by id and version only: what the
/// "already referenced" branch and the "same content" warning may show. No
/// path leaves the Ledger through it (DG3 Vault-root amendment §4.5).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceMatch {
    pub id: EvidenceReferenceId,
    pub version: AggregateVersion,
}

/// The single constant effect code every Evidence-family mutation uses.
pub(super) const EVIDENCE_EFFECT_CODE: &str = "evidence.authoritative-record-changed";

/// One Evidence reference as a Vault-root change must see it (item ⑦; the
/// accepted Vault-root amendment §3.2): where in the Vault it lives, the
/// fingerprint that can prove a file under another folder holds the same
/// content, and the version the check read it at.
///
/// `fingerprint` is `None` for a reference created before a fingerprint was
/// pinned. Such a reference cannot be proven the same under another folder,
/// which is why it blocks the change rather than being quietly assumed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceVaultEntry {
    pub id: EvidenceReferenceId,
    pub version: AggregateVersion,
    pub vault_path: VaultRelativePath,
    pub fingerprint: Option<EvidenceFingerprint>,
}

impl SqliteProductLedger {
    /// Read the current `EvidenceReferenceRecord` for `id`, or `None` if no
    /// such Evidence has been created. Read-only: opens its own transaction
    /// (mirroring `risk_repository.rs::load_risk_persistence_snapshot`) and
    /// never mutates state. `correlation_id` is used only to label any
    /// error this read itself produces (e.g. a corrupt row); it has no
    /// idempotency meaning since queries never replay.
    pub fn get_evidence_reference(
        &self,
        id: &EvidenceReferenceId,
        correlation_id: CorrelationId,
    ) -> Result<Option<EvidenceReferenceRecord>, DomainError> {
        let context = OperationContext {
            // Reused only to satisfy the shared write-path error helpers'
            // signature; queries have no idempotency semantics of their own.
            idempotency_id: IdempotencyId::parse(correlation_id.as_str()).unwrap_or_else(|_| {
                unreachable!("CorrelationId and IdempotencyId share a charset and length ceiling")
            }),
            correlation_id,
        };
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(storage_error(&context));
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| storage_error(&context))?;
        let exists: i64 = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM aggregate_registry WHERE id=?1 AND aggregate_type='evidence_reference')",
                [id.as_str()],
                |row| row.get(0),
            )
            .map_err(|_| storage_error(&context))?;
        if exists == 0 {
            return Ok(None);
        }
        let record = load_evidence_reference_record(&transaction, id, &context)?;
        transaction.commit().map_err(|_| storage_error(&context))?;
        Ok(Some(record))
    }

    /// Every Evidence reference in the Ledger, with its Vault-relative path,
    /// its pinned fingerprint and the version read — what a Vault-root
    /// change must re-resolve under the proposed folder before it can be
    /// approved (item ⑦). Read-only; ordered by id so the set the preview
    /// binds is stable and can be compared wholesale at approval.
    ///
    /// Superseded references are included: their files are still what the
    /// Ledger points at, so a folder that cannot serve them is still the
    /// wrong folder.
    pub fn list_evidence_vault_entries(&self) -> Result<Vec<EvidenceVaultEntry>, LedgerOpenError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT record.id,registry.version,record.vault_relative_path,record.fingerprint_algorithm,record.fingerprint_digest \
                 FROM evidence_references record \
                 JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='evidence_reference' \
                 ORDER BY record.id",
            )
            .map_err(super::record_entry_repository::read_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(super::record_entry_repository::read_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(super::record_entry_repository::read_error)?;
        rows.into_iter()
            .map(|(id, version, path, algorithm, digest)| {
                let fingerprint = match (algorithm, digest) {
                    (Some(algorithm), Some(digest)) => Some(EvidenceFingerprint::new(
                        FingerprintAlgorithm::from_persisted(&algorithm)
                            .map_err(|_| LedgerOpenError::StorageUnavailable)?,
                        IntegrityDigest::parse(digest)
                            .map_err(|_| LedgerOpenError::StorageUnavailable)?,
                    )),
                    // A half-written pair would mean a fingerprint that is
                    // neither present nor absent; refuse the read rather
                    // than read it as unpinned.
                    (None, None) => None,
                    _ => return Err(LedgerOpenError::StorageUnavailable),
                };
                Ok(EvidenceVaultEntry {
                    id: EvidenceReferenceId::parse(id)
                        .map_err(|_| LedgerOpenError::StorageUnavailable)?,
                    version: u64::try_from(version)
                        .ok()
                        .and_then(|value| AggregateVersion::new(value).ok())
                        .ok_or(LedgerOpenError::StorageUnavailable)?,
                    vault_path: VaultRelativePath::parse(path)
                        .map_err(|_| LedgerOpenError::StorageUnavailable)?,
                    fingerprint,
                })
            })
            .collect()
    }

    /// Evidence from a file (v48; amendment §4.5): the references that
    /// already name this Vault file. `same_name` is how Windows compares
    /// names (`pmc_platform::windows_names::same_windows_name` in the app),
    /// so two spellings that differ only in case are one file. One statement,
    /// so one consistent read; ordered by id.
    pub fn find_evidence_at_vault_path(
        &self,
        path: &VaultRelativePath,
        same_name: fn(&str, &str) -> bool,
    ) -> Result<Vec<EvidenceMatch>, LedgerOpenError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT record.id,registry.version,record.vault_relative_path \
                 FROM evidence_references record \
                 JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='evidence_reference' \
                 ORDER BY record.id",
            )
            .map_err(super::record_entry_repository::read_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(super::record_entry_repository::read_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(super::record_entry_repository::read_error)?;
        rows.into_iter()
            .filter(|(_, _, stored)| stored == path.as_str() || same_name(stored, path.as_str()))
            .map(|(id, version, _)| evidence_match(id, version))
            .collect()
    }

    /// Evidence from a file (v48; amendment §4.5): the references whose
    /// pinned fingerprint is this one — the "same content under another
    /// path" warning. Uses the partial fingerprint index; ordered by id.
    pub fn find_evidence_with_fingerprint(
        &self,
        fingerprint: &EvidenceFingerprint,
    ) -> Result<Vec<EvidenceMatch>, LedgerOpenError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT record.id,registry.version \
                 FROM evidence_references record \
                 JOIN aggregate_registry registry ON registry.id=record.id AND registry.aggregate_type='evidence_reference' \
                 WHERE record.fingerprint_algorithm=?1 AND record.fingerprint_digest=?2 \
                 ORDER BY record.id",
            )
            .map_err(super::record_entry_repository::read_error)?;
        let rows = statement
            .query_map(
                rusqlite::params![
                    fingerprint.algorithm().as_persisted(),
                    fingerprint.digest().as_str()
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .map_err(super::record_entry_repository::read_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(super::record_entry_repository::read_error)?;
        rows.into_iter()
            .map(|(id, version)| evidence_match(id, version))
            .collect()
    }

    /// Evidence from a file (v48; amendment §4.3, §4.5): create a reference
    /// for a file the host observed, with its id from the Ledger's
    /// reservation — checked again inside the create transaction, so a
    /// retry names the same reference — its fingerprint pinned and
    /// `Verified` from that one observation, `UserEntered`, and refused when
    /// another reference already names the same Vault file the way Windows
    /// compares names (`same_name`), checked under the write lock.
    #[allow(clippy::too_many_arguments)]
    pub fn create_evidence_reference_from_reservation(
        &mut self,
        reserved: &ReservedId<EvidenceReferenceId>,
        vault_path: VaultRelativePath,
        fingerprint: EvidenceFingerprint,
        observed_at: UtcTimestamp,
        classification: DataClassification,
        context: OperationContext,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
        same_name: fn(&str, &str) -> bool,
    ) -> Result<MutationOutcome<EvidenceReferenceRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&context)))?;
        if reserved.idempotency_id() != &context.idempotency_id
            || reserved.request() != EVIDENCE_RESERVATION
        {
            return Err(LedgerTransactionError::Operation(idempotency_conflict(
                &context,
            )));
        }
        let command = CreateEvidenceReference {
            id: reserved.id().clone(),
            vault_path,
            verification: EvidenceVerification::Verified {
                verified_at: observed_at,
                integrity_digest: fingerprint.digest().clone(),
            },
            fingerprint: Some(fingerprint),
            classification: Some(classification),
            provenance: Provenance::UserEntered,
            context: context.clone(),
        };
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if !reservation_matches(tx, reserved).map_err(|_| storage_error(&context))? {
                return Err(idempotency_conflict(&context));
            }
            insert_evidence_reference(
                tx,
                command,
                audit_event_id,
                occurred_at,
                expected_revision,
                Some(same_name),
            )
        })
    }

    pub fn create_evidence_reference(
        &mut self,
        command: CreateEvidenceReference,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<EvidenceReferenceRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        self.with_immediate_transaction(|transaction| {
            insert_evidence_reference(
                &mut transaction.transaction,
                command,
                audit_event_id,
                occurred_at,
                expected_revision,
                None,
            )
        })
    }

    pub fn link_evidence(
        &mut self,
        command: LinkEvidence,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<EvidenceLinkRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let target_type = target_aggregate_type(&command.target);
            let target_id = target_id_str(&command.target);
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_evidence_id,command_expected_evidence_version,command_target_type,command_target_id FROM evidence_link_command_results WHERE operation='link_evidence' AND idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.evidence_id.as_str()
                    && u64::try_from(existing.1).ok()
                        == Some(command.expected_evidence_version.get())
                    && existing.2 == target_type
                    && existing.3 == target_id;
                if matches {
                    return decode_evidence_link_outcome(tx, &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let evidence_row: Option<(i64, String)> = tx
                .query_row(
                    "SELECT version,classification FROM aggregate_registry WHERE id=?1 AND aggregate_type='evidence_reference'",
                    [command.evidence_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?;
            let Some((evidence_version, evidence_classification)) = evidence_row else {
                return Err(domain_not_found(&context));
            };
            if u64::try_from(evidence_version).ok() != Some(command.expected_evidence_version.get())
            {
                return Err(domain_conflict(&context));
            }
            if is_evidence_reference_superseded(tx, &command.evidence_id, &context)? {
                return Err(supersession_domain_error(
                    &EvidenceSupersessionError::SourceAlreadySuperseded,
                    &context,
                ));
            }
            let target_classification: Option<String> = tx
                .query_row(
                    "SELECT classification FROM aggregate_registry WHERE id=?1 AND aggregate_type=?2",
                    rusqlite::params![target_id, target_type],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| storage_error(&context))?;
            let Some(target_classification) = target_classification else {
                return Err(domain_not_found(&context));
            };
            let already_linked: i64 = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM evidence_links WHERE evidence_id=?1 AND target_type=?2 AND target_id=?3)",
                    rusqlite::params![command.evidence_id.as_str(), target_type, target_id],
                    |row| row.get(0),
                )
                .map_err(|_| storage_error(&context))?;
            if already_linked != 0 {
                return Err(domain_conflict(&context));
            }
            let classification = DataClassification::from_persisted(&evidence_classification)
                .map_err(|_| storage_error(&context))?
                .combine(
                    DataClassification::from_persisted(&target_classification)
                        .map_err(|_| storage_error(&context))?,
                );
            let occurred_millis = occurred_at.unix_millis();
            tx.execute(
                "INSERT INTO evidence_links(evidence_id,target_type,target_id,classification,linked_at) VALUES(?1,?2,?3,?4,?5)",
                rusqlite::params![
                    command.evidence_id.as_str(),
                    target_type,
                    target_id,
                    classification.as_persisted(),
                    occurred_millis,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let audit = build_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::EvidenceReference(command.evidence_id.clone()),
                "evidence.linked",
                &context,
            )?;
            persist_evidence_audit(tx, &audit, command.evidence_id.as_str(), &context)?;
            let ordinal = next_evidence_link_operation_ordinal(tx, &context)?;
            tx.execute(
                "INSERT INTO evidence_link_command_results (operation,idempotency_id,correlation_id,operation_ordinal,command_evidence_id,command_expected_evidence_version,command_target_type,command_target_id,result_classification,result_linked_at,audit_event_id) VALUES ('link_evidence',?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    command.evidence_id.as_str(),
                    i64::try_from(command.expected_evidence_version.get())
                        .map_err(|_| storage_error(&context))?,
                    target_type,
                    target_id,
                    classification.as_persisted(),
                    occurred_millis,
                    audit.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            Ok(MutationOutcome {
                record: EvidenceLinkRecord {
                    evidence_id: command.evidence_id,
                    target: command.target,
                    classification,
                    linked_at: occurred_at,
                },
                audit_event: audit,
            })
        })
    }

    pub fn update_evidence_verification(
        &mut self,
        command: UpdateEvidenceVerification,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<EvidenceReferenceRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_id,command_expected_version,command_verification,command_last_verified_at,command_integrity_digest FROM evidence_verification_command_results WHERE operation='update_evidence_verification' AND idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<i64>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.id.as_str()
                    && u64::try_from(existing.1).ok() == Some(command.expected_version.get())
                    && existing.2 == verification_kind(&command.verification)
                    && existing.3 == verification_last_verified_at(&command.verification)
                    && existing.4.as_deref() == verification_integrity_digest(&command.verification);
                if matches {
                    return decode_evidence_verification_outcome(tx, &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let current: Option<(i64, String)> = tx
                .query_row(
                    "SELECT ar.version,er.vault_relative_path FROM aggregate_registry ar JOIN evidence_references er ON er.id=ar.id WHERE ar.id=?1 AND ar.aggregate_type='evidence_reference'",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?;
            let Some((current_version, current_path)) = current else {
                return Err(domain_not_found(&context));
            };
            if u64::try_from(current_version).ok() != Some(command.expected_version.get()) {
                return Err(domain_conflict(&context));
            }
            let occurred_millis = occurred_at.unix_millis();
            let new_version = current_version + 1;
            tx.execute(
                "UPDATE evidence_references SET verification=?1,integrity_digest=?2,last_verified_at=?3 WHERE id=?4",
                rusqlite::params![
                    verification_kind(&command.verification),
                    verification_integrity_digest(&command.verification),
                    verification_last_verified_at(&command.verification),
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE aggregate_registry SET version=?1,updated_at=?2 WHERE id=?3 AND aggregate_type='evidence_reference' AND version=?4",
                    rusqlite::params![
                        new_version,
                        occurred_millis,
                        command.id.as_str(),
                        current_version
                    ],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            let audit = build_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::EvidenceReference(command.id.clone()),
                "evidence.verification_updated",
                &context,
            )?;
            persist_evidence_audit(tx, &audit, command.id.as_str(), &context)?;
            let ordinal = next_evidence_verification_operation_ordinal(tx, &context)?;
            tx.execute(
                "INSERT INTO evidence_verification_command_results (operation,idempotency_id,correlation_id,operation_ordinal,command_id,command_expected_version,command_vault_path,command_verification,command_last_verified_at,command_integrity_digest,result_version,audit_event_id) VALUES ('update_evidence_verification',?1,?2,?3,?4,?5,?11,?6,?7,?8,?9,?10)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get())
                        .map_err(|_| storage_error(&context))?,
                    verification_kind(&command.verification),
                    verification_last_verified_at(&command.verification),
                    verification_integrity_digest(&command.verification),
                    new_version,
                    audit.id().as_str(),
                    current_path,
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            let record = load_evidence_reference_record(tx, &command.id, &context)?;
            Ok(MutationOutcome {
                record,
                audit_event: audit,
            })
        })
    }

    /// Move an already-pinned Evidence reference's `vault_path` after
    /// verifying the caller's freshly-observed fingerprint at the new
    /// location still equals what was pinned at creation -- proving this is
    /// the same content, not a substitution under a trusted identity. See
    /// [`RelocateEvidenceReference`]'s own doc comment for the full
    /// rationale and its deliberate exclusion of the different-fingerprint
    /// case.
    pub fn relocate_evidence_reference(
        &mut self,
        command: RelocateEvidenceReference,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<EvidenceReferenceRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_id,command_expected_version,command_expected_current_path,command_new_vault_path,command_observed_fingerprint_algorithm,command_observed_fingerprint_digest,command_observed_at FROM evidence_relocation_command_results WHERE operation='relocate_evidence_reference' AND idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                            row.get::<_, i64>(6)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.id.as_str()
                    && u64::try_from(existing.1).ok() == Some(command.expected_version.get())
                    && existing.2 == command.expected_current_path.as_str()
                    && existing.3 == command.new_vault_path.as_str()
                    && existing.4 == command.observed_fingerprint.algorithm().as_persisted()
                    && existing.5 == command.observed_fingerprint.digest().as_str()
                    && existing.6 == command.observed_at.unix_millis();
                if matches {
                    return decode_evidence_relocation_outcome(tx, &context);
                }
                return Err(idempotency_conflict(&context));
            }
            if command.expected_current_path.as_str() == command.new_vault_path.as_str() {
                return Err(relocation_path_unchanged(&context));
            }
            let current: Option<(i64, String, Option<String>, Option<String>)> = tx
                .query_row(
                    "SELECT ar.version,er.vault_relative_path,er.fingerprint_algorithm,er.fingerprint_digest FROM aggregate_registry ar JOIN evidence_references er ON er.id=ar.id WHERE ar.id=?1 AND ar.aggregate_type='evidence_reference'",
                    [command.id.as_str()],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?;
            let Some((current_version, current_path, pinned_algorithm, pinned_digest)) = current
            else {
                return Err(domain_not_found(&context));
            };
            if u64::try_from(current_version).ok() != Some(command.expected_version.get()) {
                return Err(domain_conflict(&context));
            }
            if current_path != command.expected_current_path.as_str() {
                return Err(relocation_path_mismatch(&context));
            }
            let (Some(pinned_algorithm), Some(pinned_digest)) = (pinned_algorithm, pinned_digest)
            else {
                return Err(relocation_fingerprint_unpinned(&context));
            };
            if pinned_algorithm != command.observed_fingerprint.algorithm().as_persisted()
                || pinned_digest != command.observed_fingerprint.digest().as_str()
            {
                return Err(relocation_fingerprint_mismatch(&context));
            }
            let occurred_millis = occurred_at.unix_millis();
            let observed_millis = command.observed_at.unix_millis();
            let new_version = current_version + 1;
            tx.execute(
                "UPDATE evidence_references SET vault_relative_path=?1,verification='verified',integrity_digest=?2,last_verified_at=?3 WHERE id=?4",
                rusqlite::params![
                    command.new_vault_path.as_str(),
                    command.observed_fingerprint.digest().as_str(),
                    observed_millis,
                    command.id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE aggregate_registry SET version=?1,updated_at=?2 WHERE id=?3 AND aggregate_type='evidence_reference' AND version=?4",
                    rusqlite::params![
                        new_version,
                        occurred_millis,
                        command.id.as_str(),
                        current_version
                    ],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            let audit = build_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::EvidenceReference(command.id.clone()),
                "evidence.reference_relocated",
                &context,
            )?;
            persist_evidence_audit(tx, &audit, command.id.as_str(), &context)?;
            let ordinal = next_evidence_relocation_operation_ordinal(tx, &context)?;
            tx.execute(
                "INSERT INTO evidence_relocation_command_results (operation,idempotency_id,correlation_id,operation_ordinal,command_id,command_expected_version,command_expected_current_path,command_new_vault_path,command_observed_fingerprint_algorithm,command_observed_fingerprint_digest,command_observed_at,result_version,audit_event_id) VALUES ('relocate_evidence_reference',?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get())
                        .map_err(|_| storage_error(&context))?,
                    command.expected_current_path.as_str(),
                    command.new_vault_path.as_str(),
                    command.observed_fingerprint.algorithm().as_persisted(),
                    command.observed_fingerprint.digest().as_str(),
                    observed_millis,
                    new_version,
                    audit.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            let record = load_evidence_reference_record(tx, &command.id, &context)?;
            Ok(MutationOutcome {
                record,
                audit_event: audit,
            })
        })
    }

    /// Fingerprint pin -- pin a fingerprint onto a reference created
    /// without one (H1-User). Relocation's shape: exact replay from this
    /// command's own result row, expected-version and expected-path checks,
    /// one audit event, one Ledger revision.
    ///
    /// The reference must be *currently unpinned*. A pin is the identity and
    /// is never rewritten (same bytes: re-observe; moved: relocate; changed:
    /// supersede). The observed digest becomes both the pin and the
    /// `Verified` state's digest, deliberately: the schema couples the two
    /// pin columns to each other but not to the verification digest, so the
    /// writer has to use the one typed fingerprint for both. The Ledger
    /// cannot tell a fresh digest from a stale one; freshness is the
    /// application flow's guarantee (see `pmc_application::knowledge`).
    pub fn pin_evidence_fingerprint(
        &mut self,
        command: PinEvidenceFingerprint,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<EvidenceReferenceRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT command_id,command_expected_version,command_expected_current_path,command_observed_fingerprint_algorithm,command_observed_fingerprint_digest,command_observed_at FROM evidence_fingerprint_pin_command_results WHERE operation='pin_evidence_fingerprint' AND idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, i64>(5)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.id.as_str()
                    && u64::try_from(existing.1).ok() == Some(command.expected_version.get())
                    && existing.2 == command.expected_current_path.as_str()
                    && existing.3 == command.observed_fingerprint.algorithm().as_persisted()
                    && existing.4 == command.observed_fingerprint.digest().as_str()
                    && existing.5 == command.observed_at.unix_millis();
                if matches {
                    return decode_evidence_fingerprint_pin_outcome(tx, &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let current: Option<(i64, String, Option<String>)> = tx
                .query_row(
                    "SELECT ar.version,er.vault_relative_path,er.fingerprint_algorithm FROM aggregate_registry ar JOIN evidence_references er ON er.id=ar.id WHERE ar.id=?1 AND ar.aggregate_type='evidence_reference'",
                    [command.id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?;
            let Some((current_version, current_path, pinned_algorithm)) = current else {
                return Err(domain_not_found(&context));
            };
            if u64::try_from(current_version).ok() != Some(command.expected_version.get()) {
                return Err(domain_conflict(&context));
            }
            if current_path != command.expected_current_path.as_str() {
                return Err(pin_path_mismatch(&context));
            }
            if pinned_algorithm.is_some() {
                return Err(pin_fingerprint_already_pinned(&context));
            }
            let occurred_millis = occurred_at.unix_millis();
            let observed_millis = command.observed_at.unix_millis();
            let new_version = current_version + 1;
            // The pin and the verification digest are the same typed value,
            // and the row must still be unpinned at the moment of writing.
            if tx
                .execute(
                    "UPDATE evidence_references SET fingerprint_algorithm=?1,fingerprint_digest=?2,verification='verified',integrity_digest=?2,last_verified_at=?3 WHERE id=?4 AND fingerprint_algorithm IS NULL",
                    rusqlite::params![
                        command.observed_fingerprint.algorithm().as_persisted(),
                        command.observed_fingerprint.digest().as_str(),
                        observed_millis,
                        command.id.as_str(),
                    ],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(pin_fingerprint_already_pinned(&context));
            }
            if tx
                .execute(
                    "UPDATE aggregate_registry SET version=?1,updated_at=?2 WHERE id=?3 AND aggregate_type='evidence_reference' AND version=?4",
                    rusqlite::params![
                        new_version,
                        occurred_millis,
                        command.id.as_str(),
                        current_version
                    ],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            let audit = build_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::EvidenceReference(command.id.clone()),
                "evidence.fingerprint_pinned",
                &context,
            )?;
            persist_evidence_audit(tx, &audit, command.id.as_str(), &context)?;
            let ordinal = next_evidence_fingerprint_pin_operation_ordinal(tx, &context)?;
            tx.execute(
                "INSERT INTO evidence_fingerprint_pin_command_results (operation,idempotency_id,correlation_id,operation_ordinal,command_id,command_expected_version,command_expected_current_path,command_observed_fingerprint_algorithm,command_observed_fingerprint_digest,command_observed_at,result_version,audit_event_id) VALUES ('pin_evidence_fingerprint',?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    command.id.as_str(),
                    i64::try_from(command.expected_version.get())
                        .map_err(|_| storage_error(&context))?,
                    command.expected_current_path.as_str(),
                    command.observed_fingerprint.algorithm().as_persisted(),
                    command.observed_fingerprint.digest().as_str(),
                    observed_millis,
                    new_version,
                    audit.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            let record = load_evidence_reference_record(tx, &command.id, &context)?;
            Ok(MutationOutcome {
                record,
                audit_event: audit,
            })
        })
    }

    /// Evidence relink, replacement case -- H2a step 1. Loads the source's
    /// live state and full link set, delegates the actual preview/digest
    /// construction and validation to `pmc_domain::evidence`'s pure
    /// `prepare_supersede_evidence_reference`, then durably persists the
    /// resulting prepared intent. See `crates/pmc-domain/src/evidence.rs`'s
    /// own module-level design note for why this is its own narrow H2a
    /// mechanism rather than joining `WorkManagementOperation`.
    pub fn prepare_supersede_evidence_reference(
        &mut self,
        command: PrepareSupersedeEvidenceReference,
        prepared_intent_id: PreparedIntentId,
        now: UtcTimestamp,
    ) -> Result<EvidenceSupersessionPreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = tx
                .query_row(
                    "SELECT source_id,expected_source_version,replacement_id,replacement_vault_path,replacement_fingerprint_digest,replacement_observed_at,replacement_classification,rationale FROM evidence_h2a_command_prepare_supersessions WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, i64>(5)?,
                            row.get::<_, String>(6)?,
                            row.get::<_, String>(7)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.source_id.as_str()
                    && u64::try_from(existing.1).ok() == Some(command.expected_source_version.get())
                    && existing.2 == command.replacement_id.as_str()
                    && existing.3 == command.replacement_vault_path.as_str()
                    && existing.4 == command.replacement_fingerprint.digest().as_str()
                    && existing.5 == command.replacement_observed_at.unix_millis()
                    && existing.6 == command.replacement_classification.as_persisted()
                    && existing.7 == command.rationale.as_str();
                if matches {
                    return decode_evidence_supersession_prepare_outcome(tx, &context);
                }
                return Err(idempotency_conflict(&context));
            }
            let snapshot = load_evidence_supersession_source_snapshot(tx, &command.source_id, &context)?;
            if is_evidence_reference_superseded(tx, &command.source_id, &context)? {
                return Err(supersession_domain_error(
                    &EvidenceSupersessionError::SourceAlreadySuperseded,
                    &context,
                ));
            }
            let prepared = prepare_supersession(&command, prepared_intent_id, &snapshot, now)
                .map_err(|error| supersession_domain_error(&error, &context))?;
            persist_evidence_supersession_prepared_intent(tx, &prepared, &context)?;
            let ordinal = next_evidence_h2a_supersession_prepare_operation_ordinal(tx, &context)?;
            tx.execute(
                "INSERT INTO evidence_h2a_supersession_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_supersede_evidence_reference',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO evidence_h2a_command_prepare_supersessions (idempotency_id,source_id,expected_source_version,replacement_id,replacement_vault_path,replacement_fingerprint_algorithm,replacement_fingerprint_digest,replacement_observed_at,replacement_classification,replacement_provenance_kind,replacement_provenance_reference,rationale) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.source_id.as_str(),
                    i64::try_from(command.expected_source_version.get())
                        .map_err(|_| storage_error(&context))?,
                    command.replacement_id.as_str(),
                    command.replacement_vault_path.as_str(),
                    command.replacement_fingerprint.algorithm().as_persisted(),
                    command.replacement_fingerprint.digest().as_str(),
                    command.replacement_observed_at.unix_millis(),
                    command.replacement_classification.as_persisted(),
                    command.replacement_provenance.kind_persisted(),
                    command.replacement_provenance.reference().map(ProvenanceReference::as_str),
                    command.rationale.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            Ok(prepared)
        })
    }

    /// Evidence relink, replacement case -- H2a step 2. Reloads the prepared
    /// intent and the source's *current* live snapshot, delegates
    /// re-validation and effect construction to
    /// `execute_supersede_evidence_reference`, then atomically creates the
    /// replacement, clones every link onto it, marks the source superseded,
    /// records the supersession edge, mints the Approval Receipt, and
    /// audits every effect.
    pub fn approve_and_execute_supersede_evidence_reference(
        &mut self,
        command: ApproveAndExecuteSupersedeEvidenceReference,
        approval_receipt_id: ApprovalReceiptId,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<MutationOutcome<EvidenceReferenceRecord>, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if command.approval.idempotency_id() != &context.idempotency_id {
                return Err(idempotency_conflict(&context));
            }
            if let Some(existing) = tx
                .query_row(
                    "SELECT prepared_id,actor,acknowledged_digest FROM evidence_h2a_command_execute_supersessions WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = existing.0 == command.approval.prepared_id().as_str()
                    && existing.1 == command.approval.actor().as_persisted()
                    && existing.2 == command.approval.acknowledged_payload_digest().as_str();
                if !matches {
                    return Err(idempotency_conflict(&context));
                }
                return decode_evidence_supersession_execute_outcome(tx, &context);
            }
            let prepared =
                load_evidence_supersession_prepared_intent(tx, command.approval.prepared_id(), &context)?;
            let current = load_evidence_supersession_source_snapshot(
                tx,
                &prepared.preview().source_id,
                &context,
            )?;
            let already_superseded =
                is_evidence_reference_superseded(tx, &prepared.preview().source_id, &context)?;
            let effects = execute_supersede_evidence_reference(
                &command.approval,
                &prepared,
                &current,
                already_superseded,
                occurred_at,
            )
            .map_err(|error| supersession_domain_error(&error, &context))?;
            if tx
                .execute(
                    "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
                    rusqlite::params![occurred_at.unix_millis(), prepared.id().as_str()],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(idempotency_conflict(&context));
            }
            tx.execute(
                "INSERT INTO approval_receipts (id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                rusqlite::params![
                    approval_receipt_id.as_str(),
                    prepared.id().as_str(),
                    command.approval.actor().as_persisted(),
                    command.approval.acknowledged_payload_digest().as_str(),
                    context.idempotency_id.as_str(),
                    occurred_at.unix_millis(),
                    prepared.preview().expires_at.unix_millis(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_evidence_supersession_effects(
                tx,
                &effects,
                prepared.id(),
                &approval_receipt_id,
                &prepared.preview().rationale,
                occurred_at,
                &context,
            )?;
            let audit = build_evidence_supersession_audit(audit_event_id, occurred_at, &effects, &context)?;
            persist_evidence_supersession_audit(tx, &audit, &effects, &context)?;
            let ordinal = next_evidence_h2a_supersession_execute_operation_ordinal(tx, &context)?;
            tx.execute(
                "INSERT INTO evidence_h2a_supersession_execute_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,source_id,replacement_id,prepared_intent_id,approval_receipt_id) VALUES (?1,'execute_supersede_evidence_reference',?2,?3,'superseded',?4,?5,?6,?7)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    effects.source_id.as_str(),
                    effects.replacement.id.as_str(),
                    prepared.id().as_str(),
                    approval_receipt_id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO evidence_h2a_command_execute_supersessions (idempotency_id,prepared_id,actor,acknowledged_digest) VALUES (?1,?2,?3,?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.approval.prepared_id().as_str(),
                    command.approval.actor().as_persisted(),
                    command.approval.acknowledged_payload_digest().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO evidence_h2a_supersession_execute_replay_audits (idempotency_id,ordinal,audit_event_id,correlation_id) VALUES (?1,0,?2,?3)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    audit.id().as_str(),
                    context.correlation_id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let expected_revision =
                i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
            if tx
                .execute(
                    "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                    rusqlite::params![expected_revision + 1, expected_revision],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(storage_error(&context));
            }
            Ok(MutationOutcome {
                record: effects.replacement,
                audit_event: audit,
            })
        })
    }
}

/// Load the source's current record plus its complete, live link set (each
/// carrying the linked target's own current version/classification, not
/// just the link row's own recorded classification) -- the authoritative
/// snapshot both `prepare_supersede_evidence_reference` and
/// `execute_supersede_evidence_reference` validate against. Returns
/// `domain_not_found` if `source_id` names no Evidence reference at all.
fn load_evidence_supersession_source_snapshot(
    tx: &Transaction<'_>,
    source_id: &EvidenceReferenceId,
    context: &OperationContext,
) -> Result<EvidenceSupersessionSourceSnapshot, DomainError> {
    let exists: i64 = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM aggregate_registry WHERE id=?1 AND aggregate_type='evidence_reference')",
            [source_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    if exists == 0 {
        return Err(domain_not_found(context));
    }
    let record = load_evidence_reference_record(tx, source_id, context)?;
    let link_rows: Vec<(String, String, String, i64)> = tx
        .prepare(
            "SELECT target_type,target_id,classification,linked_at FROM evidence_links WHERE evidence_id=?1",
        )
        .map_err(|_| storage_error(context))?
        .query_map([source_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|_| storage_error(context))?
        .collect::<Result<_, _>>()
        .map_err(|_| storage_error(context))?;
    let mut links = Vec::with_capacity(link_rows.len());
    for (target_type, target_id, link_classification, linked_at) in link_rows {
        let (target_version, target_classification): (i64, String) = tx
            .query_row(
                "SELECT version,classification FROM aggregate_registry WHERE id=?1 AND aggregate_type=?2",
                rusqlite::params![target_id, target_type],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| storage_error(context))?;
        links.push(EvidenceSupersessionLinkSnapshot {
            target: decode_target(&target_type, target_id, context)?,
            target_version: AggregateVersion::new(
                u64::try_from(target_version).map_err(|_| storage_error(context))?,
            )
            .map_err(|_| storage_error(context))?,
            target_classification: DataClassification::from_persisted(&target_classification)
                .map_err(|_| storage_error(context))?,
            link_classification: DataClassification::from_persisted(&link_classification)
                .map_err(|_| storage_error(context))?,
            linked_at: UtcTimestamp::from_unix_millis(linked_at),
        });
    }
    Ok(EvidenceSupersessionSourceSnapshot { record, links })
}

fn is_evidence_reference_superseded(
    tx: &Transaction<'_>,
    source_id: &EvidenceReferenceId,
    context: &OperationContext,
) -> Result<bool, DomainError> {
    let exists: i64 = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM evidence_reference_supersessions WHERE source_id=?1)",
            [source_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    Ok(exists != 0)
}

fn supersession_domain_error(
    error: &EvidenceSupersessionError,
    context: &OperationContext,
) -> DomainError {
    let (code, key) = match error {
        EvidenceSupersessionError::SourceMismatch => (
            ErrorCode::DomainConflict,
            "evidence.supersession_source_mismatch",
        ),
        EvidenceSupersessionError::ReplacementIsSource => (
            ErrorCode::DomainConflict,
            "evidence.supersession_replacement_is_source",
        ),
        EvidenceSupersessionError::NotAGenuineReplacement => (
            ErrorCode::DomainConflict,
            "evidence.supersession_not_a_genuine_replacement",
        ),
        EvidenceSupersessionError::UnclassifiedReplacement => (
            ErrorCode::DomainConflict,
            "evidence.supersession_unclassified_replacement",
        ),
        EvidenceSupersessionError::ReplacementLowersClassification => (
            ErrorCode::DomainConflict,
            "evidence.supersession_lowers_classification",
        ),
        EvidenceSupersessionError::SourceAlreadySuperseded => (
            ErrorCode::DomainConflict,
            "evidence.supersession_source_already_superseded",
        ),
        EvidenceSupersessionError::DigestMismatch => (
            ErrorCode::SecurityPreviewExpiredOrChanged,
            "evidence.supersession_digest_mismatch",
        ),
        EvidenceSupersessionError::Expired => (
            ErrorCode::SecurityPreviewExpiredOrChanged,
            "evidence.supersession_expired",
        ),
        EvidenceSupersessionError::PreviewChanged => (
            ErrorCode::SecurityPreviewExpiredOrChanged,
            "evidence.supersession_preview_changed",
        ),
        EvidenceSupersessionError::UnauthorizedActor => (
            ErrorCode::SecurityPolicyDenied,
            "evidence.supersession_unauthorized_actor",
        ),
        EvidenceSupersessionError::MissingConfirmation => (
            ErrorCode::SecurityPolicyDenied,
            "evidence.supersession_missing_confirmation",
        ),
        EvidenceSupersessionError::PreparedIntentMismatch => (
            ErrorCode::SecurityPreviewExpiredOrChanged,
            "evidence.supersession_prepared_intent_mismatch",
        ),
    };
    DomainError::new(
        code,
        MessageKey::parse(key).unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn next_evidence_h2a_supersession_prepare_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM evidence_h2a_supersession_prepare_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn next_evidence_h2a_supersession_execute_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM evidence_h2a_supersession_execute_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn persist_evidence_supersession_prepared_intent(
    tx: &Transaction<'_>,
    prepared: &EvidenceSupersessionPreparedIntent,
    context: &OperationContext,
) -> Result<(), DomainError> {
    let preview = prepared.preview();
    tx.execute(
        "INSERT INTO prepared_intents (id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,expires_at,created_at) VALUES (?1,1,'supersede_evidence_reference',?2,?3,'allowed','not_cancellable_after_submit','head_of_products',?4,?5)",
        rusqlite::params![
            prepared.id().as_str(),
            prepared.payload_digest().as_str(),
            preview.classification.as_persisted(),
            preview.expires_at.unix_millis(),
            prepared.created_at().unix_millis(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO prepared_intent_targets (prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES (?1,0,'evidence_reference',?2,?3)",
        rusqlite::params![
            prepared.id().as_str(),
            preview.source_id.as_str(),
            i64::try_from(preview.source_version.get()).map_err(|_| storage_error(context))?,
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO prepared_intent_classification_sources (prepared_intent_id,ordinal,role,source_id,classification) VALUES (?1,0,'primary_target',?2,?3)",
        rusqlite::params![
            prepared.id().as_str(),
            preview.source_id.as_str(),
            preview.source_classification.as_persisted()
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO prepared_intent_classification_sources (prepared_intent_id,ordinal,role,source_id,classification) VALUES (?1,1,'replacement_evidence',?2,?3)",
        rusqlite::params![
            prepared.id().as_str(),
            preview.replacement_id.as_str(),
            preview.replacement_classification.as_persisted()
        ],
    )
    .map_err(|_| storage_error(context))?;
    for (index, link) in preview.links.iter().enumerate() {
        let ordinal = i64::try_from(index + 2).map_err(|_| storage_error(context))?;
        tx.execute(
            "INSERT INTO prepared_intent_classification_sources (prepared_intent_id,ordinal,role,source_id,classification) VALUES (?1,?2,'linked_target',?3,?4)",
            rusqlite::params![
                prepared.id().as_str(),
                ordinal,
                target_id_str(&link.target),
                link.target_classification.as_persisted()
            ],
        )
        .map_err(|_| storage_error(context))?;
    }
    tx.execute(
        "INSERT INTO prepared_evidence_supersession_payloads (prepared_intent_id,source_id,source_version,source_vault_path,source_fingerprint_algorithm,source_fingerprint_digest,source_classification,replacement_id,replacement_vault_path,replacement_fingerprint_algorithm,replacement_fingerprint_digest,replacement_observed_at,replacement_classification,replacement_provenance_kind,replacement_provenance_reference,classification,rationale) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)",
        rusqlite::params![
            prepared.id().as_str(),
            preview.source_id.as_str(),
            i64::try_from(preview.source_version.get()).map_err(|_| storage_error(context))?,
            preview.source_vault_path.as_str(),
            preview.source_fingerprint.as_ref().map(|value| value.algorithm().as_persisted()),
            preview.source_fingerprint.as_ref().map(|value| value.digest().as_str()),
            preview.source_classification.as_persisted(),
            preview.replacement_id.as_str(),
            preview.replacement_vault_path.as_str(),
            preview.replacement_fingerprint.algorithm().as_persisted(),
            preview.replacement_fingerprint.digest().as_str(),
            preview.replacement_observed_at.unix_millis(),
            preview.replacement_classification.as_persisted(),
            preview.replacement_provenance.kind_persisted(),
            preview.replacement_provenance.reference().map(ProvenanceReference::as_str),
            preview.classification.as_persisted(),
            preview.rationale.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    for (index, link) in preview.links.iter().enumerate() {
        let ordinal = i64::try_from(index).map_err(|_| storage_error(context))?;
        tx.execute(
            "INSERT INTO prepared_evidence_supersession_links (prepared_intent_id,ordinal,target_type,target_id,target_version,target_classification,link_classification,linked_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![
                prepared.id().as_str(),
                ordinal,
                target_aggregate_type(&link.target),
                target_id_str(&link.target),
                i64::try_from(link.target_version.get()).map_err(|_| storage_error(context))?,
                link.target_classification.as_persisted(),
                link.link_classification.as_persisted(),
                link.linked_at.unix_millis(),
            ],
        )
        .map_err(|_| storage_error(context))?;
    }
    Ok(())
}

fn decode_evidence_supersession_prepare_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<EvidenceSupersessionPreparedIntent, DomainError> {
    let prepared_id: String = tx
        .query_row(
            "SELECT result_reference FROM evidence_h2a_supersession_prepare_replay_operations WHERE idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let prepared_id = PreparedIntentId::parse(prepared_id).map_err(|_| storage_error(context))?;
    load_evidence_supersession_prepared_intent(tx, &prepared_id, context)
}

/// Reload a durably persisted `EvidenceSupersessionPreparedIntent` exactly
/// as it was prepared -- used both to decode a `prepare` replay and to feed
/// `execute_supersede_evidence_reference` the prepared intent it must
/// re-validate against a freshly loaded current snapshot.
fn load_evidence_supersession_prepared_intent(
    tx: &Transaction<'_>,
    prepared_id: &PreparedIntentId,
    context: &OperationContext,
) -> Result<EvidenceSupersessionPreparedIntent, DomainError> {
    let (payload_digest, expires_at, created_at): (String, i64, i64) = tx
        .query_row(
            "SELECT payload_digest,expires_at,created_at FROM prepared_intents WHERE id=?1 AND intent_kind='supersede_evidence_reference'",
            [prepared_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| storage_error(context))?;
    let payload = tx
        .query_row(
            "SELECT source_id,source_version,source_vault_path,source_fingerprint_algorithm,source_fingerprint_digest,source_classification,replacement_id,replacement_vault_path,replacement_fingerprint_algorithm,replacement_fingerprint_digest,replacement_observed_at,replacement_classification,replacement_provenance_kind,replacement_provenance_reference,classification,rationale FROM prepared_evidence_supersession_payloads WHERE prepared_intent_id=?1",
            [prepared_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, String>(15)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let link_rows: Vec<(String, String, i64, String, String, i64)> = tx
        .prepare(
            "SELECT target_type,target_id,target_version,target_classification,link_classification,linked_at FROM prepared_evidence_supersession_links WHERE prepared_intent_id=?1 ORDER BY ordinal",
        )
        .map_err(|_| storage_error(context))?
        .query_map([prepared_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|_| storage_error(context))?
        .collect::<Result<_, _>>()
        .map_err(|_| storage_error(context))?;
    let mut links = Vec::with_capacity(link_rows.len());
    for (
        target_type,
        target_id,
        target_version,
        target_classification,
        link_classification,
        linked_at,
    ) in link_rows
    {
        links.push(EvidenceSupersessionLinkSnapshot {
            target: decode_target(&target_type, target_id, context)?,
            target_version: AggregateVersion::new(
                u64::try_from(target_version).map_err(|_| storage_error(context))?,
            )
            .map_err(|_| storage_error(context))?,
            target_classification: DataClassification::from_persisted(&target_classification)
                .map_err(|_| storage_error(context))?,
            link_classification: DataClassification::from_persisted(&link_classification)
                .map_err(|_| storage_error(context))?,
            linked_at: UtcTimestamp::from_unix_millis(linked_at),
        });
    }
    let source_fingerprint = match (payload.3, payload.4) {
        (Some(algorithm), Some(digest)) => Some(EvidenceFingerprint::new(
            FingerprintAlgorithm::from_persisted(&algorithm).map_err(|_| storage_error(context))?,
            IntegrityDigest::parse(digest).map_err(|_| storage_error(context))?,
        )),
        (None, None) => None,
        _ => return Err(storage_error(context)),
    };
    let preview = pmc_domain::evidence::EvidenceSupersessionPreview {
        prepared_intent_id: prepared_id.clone(),
        source_id: EvidenceReferenceId::parse(payload.0).map_err(|_| storage_error(context))?,
        source_version: AggregateVersion::new(
            u64::try_from(payload.1).map_err(|_| storage_error(context))?,
        )
        .map_err(|_| storage_error(context))?,
        source_vault_path: VaultRelativePath::parse(payload.2)
            .map_err(|_| storage_error(context))?,
        source_fingerprint,
        source_classification: DataClassification::from_persisted(&payload.5)
            .map_err(|_| storage_error(context))?,
        replacement_id: EvidenceReferenceId::parse(payload.6)
            .map_err(|_| storage_error(context))?,
        replacement_vault_path: VaultRelativePath::parse(payload.7)
            .map_err(|_| storage_error(context))?,
        replacement_fingerprint: EvidenceFingerprint::new(
            FingerprintAlgorithm::from_persisted(&payload.8).map_err(|_| storage_error(context))?,
            IntegrityDigest::parse(payload.9).map_err(|_| storage_error(context))?,
        ),
        replacement_observed_at: UtcTimestamp::from_unix_millis(payload.10),
        replacement_classification: DataClassification::from_persisted(&payload.11)
            .map_err(|_| storage_error(context))?,
        replacement_provenance: decode_provenance(&payload.12, payload.13, context)?,
        links,
        classification: DataClassification::from_persisted(&payload.14)
            .map_err(|_| storage_error(context))?,
        rationale: WorkManagementRationale::parse(payload.15)
            .map_err(|_| storage_error(context))?,
        expires_at: UtcTimestamp::from_unix_millis(expires_at),
    };
    let digest = EvidenceSupersessionPayloadDigest::from_persisted(payload_digest)
        .map_err(|_| storage_error(context))?;
    Ok(EvidenceSupersessionPreparedIntent::from_persisted(
        preview,
        digest,
        UtcTimestamp::from_unix_millis(created_at),
    ))
}

fn persist_evidence_supersession_effects(
    tx: &Transaction<'_>,
    effects: &pmc_domain::evidence::EvidenceSupersessionEffects,
    prepared_intent_id: &PreparedIntentId,
    approval_receipt_id: &ApprovalReceiptId,
    rationale: &WorkManagementRationale,
    occurred_at: UtcTimestamp,
    context: &OperationContext,
) -> Result<(), DomainError> {
    let occurred_millis = occurred_at.unix_millis();
    tx.execute(
        "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'evidence_reference',1,?2,?3,?3)",
        rusqlite::params![
            effects.replacement.id.as_str(),
            effects.replacement.classification.as_persisted(),
            occurred_millis
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO evidence_references(id,role,verification,integrity_digest,last_verified_at,vault_relative_path,fingerprint_algorithm,fingerprint_digest,provenance_kind,provenance_reference) VALUES(?1,NULL,?2,?3,?4,?5,?6,?7,?8,?9)",
        rusqlite::params![
            effects.replacement.id.as_str(),
            verification_kind(&effects.replacement.verification),
            verification_integrity_digest(&effects.replacement.verification),
            verification_last_verified_at(&effects.replacement.verification),
            effects.replacement.vault_path.as_str(),
            effects.replacement.fingerprint.as_ref().map(|value| value.algorithm().as_persisted()),
            effects.replacement.fingerprint.as_ref().map(|value| value.digest().as_str()),
            effects.replacement.provenance.kind_persisted(),
            effects.replacement.provenance.reference().map(ProvenanceReference::as_str),
        ],
    )
    .map_err(|_| storage_error(context))?;
    for link in &effects.replacement_links {
        tx.execute(
            "INSERT INTO evidence_links(evidence_id,target_type,target_id,classification,linked_at) VALUES(?1,?2,?3,?4,?5)",
            rusqlite::params![
                link.evidence_id.as_str(),
                target_aggregate_type(&link.target),
                target_id_str(&link.target),
                link.classification.as_persisted(),
                link.linked_at.unix_millis(),
            ],
        )
        .map_err(|_| storage_error(context))?;
    }
    let source_previous_version = effects
        .source_new_version
        .get()
        .checked_sub(1)
        .ok_or_else(|| storage_error(context))?;
    if tx
        .execute(
            "UPDATE aggregate_registry SET version=?1,updated_at=?2 WHERE id=?3 AND aggregate_type='evidence_reference' AND version=?4",
            rusqlite::params![
                i64::try_from(effects.source_new_version.get()).map_err(|_| storage_error(context))?,
                occurred_millis,
                effects.source_id.as_str(),
                i64::try_from(source_previous_version).map_err(|_| storage_error(context))?,
            ],
        )
        .map_err(|_| storage_error(context))?
        != 1
    {
        return Err(storage_error(context));
    }
    tx.execute(
        "INSERT INTO evidence_reference_supersessions(source_id,replacement_id,source_new_version,occurred_at,rationale,approval_receipt_id,prepared_intent_id) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![
            effects.source_id.as_str(),
            effects.replacement.id.as_str(),
            i64::try_from(effects.source_new_version.get()).map_err(|_| storage_error(context))?,
            occurred_millis,
            rationale.as_str(),
            approval_receipt_id.as_str(),
            prepared_intent_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

fn build_evidence_supersession_audit(
    id: AuditEventId,
    at: UtcTimestamp,
    effects: &pmc_domain::evidence::EvidenceSupersessionEffects,
    context: &OperationContext,
) -> Result<AuditEvent, DomainError> {
    let effect_count = 2 + effects.replacement_links.len();
    Ok(AuditEvent::new(
        id,
        at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse("evidence.reference_superseded")
                .map_err(|_| storage_error(context))?,
            AuditTarget::EvidenceReference(effects.source_id.clone()),
        ),
        context.correlation_id.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::Allowed,
            AuditApprovalOutcome::Approved,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            std::iter::repeat_n(
                AuditEffectCode::parse(EVIDENCE_EFFECT_CODE).map_err(|_| storage_error(context))?,
                effect_count,
            )
            .collect(),
        )
        .map_err(|_| storage_error(context))?,
    ))
}

fn persist_evidence_supersession_audit(
    tx: &Transaction<'_>,
    audit: &AuditEvent,
    effects: &pmc_domain::evidence::EvidenceSupersessionEffects,
    context: &OperationContext,
) -> Result<(), DomainError> {
    tx.execute(
        "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','work_management',?3,'evidence_reference',?4,?5,?6,?7,?8,?9)",
        rusqlite::params![
            audit.id().as_str(),
            audit.occurred_at().unix_millis(),
            audit.code().as_str(),
            effects.source_id.as_str(),
            context.correlation_id.as_str(),
            AuditPolicyOutcome::Allowed.as_persisted(),
            AuditApprovalOutcome::Approved.as_persisted(),
            AuditExecutionOutcome::Succeeded.as_persisted(),
            AuditEffectScope::Complete.as_persisted(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    let mut ordinal: i64 = 0;
    tx.execute(
        "INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,?2,?3,'complete','evidence_reference',?4)",
        rusqlite::params![audit.id().as_str(), ordinal, EVIDENCE_EFFECT_CODE, effects.replacement.id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    ordinal += 1;
    tx.execute(
        "INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,?2,?3,'complete','evidence_reference',?4)",
        rusqlite::params![audit.id().as_str(), ordinal, EVIDENCE_EFFECT_CODE, effects.source_id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    for link in &effects.replacement_links {
        ordinal += 1;
        tx.execute(
            "INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,?2,?3,'complete',?4,?5)",
            rusqlite::params![
                audit.id().as_str(),
                ordinal,
                EVIDENCE_EFFECT_CODE,
                target_aggregate_type(&link.target),
                target_id_str(&link.target),
            ],
        )
        .map_err(|_| storage_error(context))?;
    }
    Ok(())
}

fn decode_evidence_supersession_execute_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<MutationOutcome<EvidenceReferenceRecord>, DomainError> {
    let (replacement_id, audit_event_id): (String, String) = tx
        .query_row(
            "SELECT replacement_id,(SELECT audit_event_id FROM evidence_h2a_supersession_execute_replay_audits WHERE idempotency_id=?1 AND ordinal=0) FROM evidence_h2a_supersession_execute_replay_operations WHERE idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| storage_error(context))?;
    let replacement_id =
        EvidenceReferenceId::parse(replacement_id).map_err(|_| storage_error(context))?;
    let record = load_evidence_reference_record(tx, &replacement_id, context)?;
    let audit_event = decode_audit(tx, &audit_event_id, context)?;
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

fn target_aggregate_type(target: &EvidenceLinkTarget) -> &'static str {
    match target {
        EvidenceLinkTarget::Product(_) => "product",
        EvidenceLinkTarget::Initiative(_) => "initiative",
        EvidenceLinkTarget::Project(_) => "project",
        EvidenceLinkTarget::Roadmap(_) => "roadmap",
        EvidenceLinkTarget::Milestone(_) => "milestone",
        EvidenceLinkTarget::Kpi(_) => "kpi_definition",
        EvidenceLinkTarget::KpiObservation(_) => "kpi_observation",
        EvidenceLinkTarget::ActionRequest(_) => "action_request",
        EvidenceLinkTarget::Action(_) => "action",
        EvidenceLinkTarget::DecisionRequest(_) => "decision_request",
        EvidenceLinkTarget::Decision(_) => "decision",
        EvidenceLinkTarget::Risk(_) => "risk",
        EvidenceLinkTarget::Issue(_) => "issue",
    }
}

fn target_id_str(target: &EvidenceLinkTarget) -> &str {
    match target {
        EvidenceLinkTarget::Product(id) => id.as_str(),
        EvidenceLinkTarget::Initiative(id) => id.as_str(),
        EvidenceLinkTarget::Project(id) => id.as_str(),
        EvidenceLinkTarget::Roadmap(id) => id.as_str(),
        EvidenceLinkTarget::Milestone(id) => id.as_str(),
        EvidenceLinkTarget::Kpi(id) => id.as_str(),
        EvidenceLinkTarget::KpiObservation(id) => id.as_str(),
        EvidenceLinkTarget::ActionRequest(id) => id.as_str(),
        EvidenceLinkTarget::Action(id) => id.as_str(),
        EvidenceLinkTarget::DecisionRequest(id) => id.as_str(),
        EvidenceLinkTarget::Decision(id) => id.as_str(),
        EvidenceLinkTarget::Risk(id) => id.as_str(),
        EvidenceLinkTarget::Issue(id) => id.as_str(),
    }
}

fn decode_target(
    target_type: &str,
    target_id: String,
    context: &OperationContext,
) -> Result<EvidenceLinkTarget, DomainError> {
    match target_type {
        "product" => Ok(EvidenceLinkTarget::Product(
            ProductId::parse(target_id).map_err(|_| storage_error(context))?,
        )),
        "initiative" => Ok(EvidenceLinkTarget::Initiative(
            InitiativeId::parse(target_id).map_err(|_| storage_error(context))?,
        )),
        "project" => Ok(EvidenceLinkTarget::Project(
            ProjectId::parse(target_id).map_err(|_| storage_error(context))?,
        )),
        "roadmap" => Ok(EvidenceLinkTarget::Roadmap(
            RoadmapId::parse(target_id).map_err(|_| storage_error(context))?,
        )),
        "milestone" => Ok(EvidenceLinkTarget::Milestone(
            MilestoneId::parse(target_id).map_err(|_| storage_error(context))?,
        )),
        "kpi_definition" => Ok(EvidenceLinkTarget::Kpi(
            KpiId::parse(target_id).map_err(|_| storage_error(context))?,
        )),
        "kpi_observation" => Ok(EvidenceLinkTarget::KpiObservation(
            KpiObservationId::parse(target_id).map_err(|_| storage_error(context))?,
        )),
        "action_request" => Ok(EvidenceLinkTarget::ActionRequest(
            ActionRequestId::parse(target_id).map_err(|_| storage_error(context))?,
        )),
        "action" => Ok(EvidenceLinkTarget::Action(
            ActionId::parse(target_id).map_err(|_| storage_error(context))?,
        )),
        "decision_request" => Ok(EvidenceLinkTarget::DecisionRequest(
            DecisionRequestId::parse(target_id).map_err(|_| storage_error(context))?,
        )),
        "decision" => Ok(EvidenceLinkTarget::Decision(
            DecisionId::parse(target_id).map_err(|_| storage_error(context))?,
        )),
        "risk" => Ok(EvidenceLinkTarget::Risk(
            RiskId::parse(target_id).map_err(|_| storage_error(context))?,
        )),
        "issue" => Ok(EvidenceLinkTarget::Issue(
            IssueId::parse(target_id).map_err(|_| storage_error(context))?,
        )),
        _ => Err(storage_error(context)),
    }
}

fn decode_evidence_link_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<MutationOutcome<EvidenceLinkRecord>, DomainError> {
    let row = tx
        .query_row(
            "SELECT command_evidence_id,command_target_type,command_target_id,result_classification,result_linked_at,audit_event_id FROM evidence_link_command_results WHERE operation='link_evidence' AND idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let record = EvidenceLinkRecord {
        evidence_id: EvidenceReferenceId::parse(row.0).map_err(|_| storage_error(context))?,
        target: decode_target(&row.1, row.2, context)?,
        classification: DataClassification::from_persisted(&row.3)
            .map_err(|_| storage_error(context))?,
        linked_at: UtcTimestamp::from_unix_millis(row.4),
    };
    let audit_event = decode_audit(tx, &row.5, context)?;
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

fn next_evidence_link_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,1) FROM evidence_link_command_results",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn next_evidence_verification_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,1) FROM evidence_verification_command_results",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn next_evidence_relocation_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,1) FROM evidence_relocation_command_results",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn next_evidence_fingerprint_pin_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,1) FROM evidence_fingerprint_pin_command_results",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// Load the full *current* `EvidenceReferenceRecord` for `id` by joining
/// `aggregate_registry` (version/classification/timestamps) with
/// `evidence_references` (everything else). Only valid for callers that
/// actually want live current state (`get_evidence_reference`, and the
/// fresh-write path in `update_evidence_verification` where current state
/// and the just-produced outcome are the same row). Do NOT use this to
/// decode a *replayed* outcome: `version`/`verification`/`updated_at` all
/// change on later writes, so re-reading current state for an old
/// idempotency key would silently return a newer outcome than the one that
/// key originally produced. See [`load_evidence_reference_immutable_fields`]
/// plus the command's own stored result columns for that case.
fn load_evidence_reference_record(
    tx: &Transaction<'_>,
    id: &EvidenceReferenceId,
    context: &OperationContext,
) -> Result<EvidenceReferenceRecord, DomainError> {
    let row = tx
        .query_row(
            "SELECT ar.version,ar.classification,ar.created_at,ar.updated_at,er.vault_relative_path,er.fingerprint_algorithm,er.fingerprint_digest,er.verification,er.last_verified_at,er.integrity_digest,er.provenance_kind,er.provenance_reference FROM aggregate_registry ar JOIN evidence_references er ON er.id=ar.id WHERE ar.id=?1 AND ar.aggregate_type='evidence_reference'",
            [id.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<i64>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, Option<String>>(11)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let fingerprint = match (row.5, row.6) {
        (Some(algorithm), Some(digest)) => Some(EvidenceFingerprint::new(
            FingerprintAlgorithm::from_persisted(&algorithm).map_err(|_| storage_error(context))?,
            IntegrityDigest::parse(digest).map_err(|_| storage_error(context))?,
        )),
        (None, None) => None,
        _ => return Err(storage_error(context)),
    };
    Ok(EvidenceReferenceRecord {
        id: id.clone(),
        vault_path: VaultRelativePath::parse(row.4).map_err(|_| storage_error(context))?,
        fingerprint,
        verification: decode_verification(&row.7, row.8, row.9, context)?,
        classification: DataClassification::from_persisted(&row.1)
            .map_err(|_| storage_error(context))?,
        provenance: decode_provenance(&row.10, row.11, context)?,
        version: AggregateVersion::new(u64::try_from(row.0).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
        created_at: UtcTimestamp::from_unix_millis(row.2),
        updated_at: UtcTimestamp::from_unix_millis(row.3),
    })
}

/// Fields on `EvidenceReferenceRecord` that never change once
/// `create_evidence_reference` commits them. Safe to re-read from current
/// state at any later point, including while decoding a replayed outcome.
/// The vault path is deliberately not among them: relocation moves
/// it, so every replay decoder rebuilds it from its own command row.
struct EvidenceReferenceImmutableFields {
    fingerprint: Option<EvidenceFingerprint>,
    classification: DataClassification,
    provenance: Provenance,
    created_at: UtcTimestamp,
}

fn load_evidence_reference_immutable_fields(
    tx: &Transaction<'_>,
    id: &EvidenceReferenceId,
    context: &OperationContext,
) -> Result<EvidenceReferenceImmutableFields, DomainError> {
    let row = tx
        .query_row(
            "SELECT ar.classification,ar.created_at,er.fingerprint_algorithm,er.fingerprint_digest,er.provenance_kind,er.provenance_reference FROM aggregate_registry ar JOIN evidence_references er ON er.id=ar.id WHERE ar.id=?1 AND ar.aggregate_type='evidence_reference'",
            [id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let fingerprint = match (row.2, row.3) {
        (Some(algorithm), Some(digest)) => Some(EvidenceFingerprint::new(
            FingerprintAlgorithm::from_persisted(&algorithm).map_err(|_| storage_error(context))?,
            IntegrityDigest::parse(digest).map_err(|_| storage_error(context))?,
        )),
        (None, None) => None,
        _ => return Err(storage_error(context)),
    };
    Ok(EvidenceReferenceImmutableFields {
        fingerprint,
        classification: DataClassification::from_persisted(&row.0)
            .map_err(|_| storage_error(context))?,
        provenance: decode_provenance(&row.4, row.5, context)?,
        created_at: UtcTimestamp::from_unix_millis(row.1),
    })
}

/// Decode the exact outcome `update_evidence_verification` produced for a
/// given idempotency key -- not whatever current state now is. `version`,
/// `verification`, `updated_at` and (since v45) `vault_path` are
/// reconstructed from that command's own stored columns; only the
/// genuinely immutable fields are re-read from current state.
fn decode_evidence_verification_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<MutationOutcome<EvidenceReferenceRecord>, DomainError> {
    let row = tx
        .query_row(
            "SELECT command_id,command_verification,command_last_verified_at,command_integrity_digest,result_version,audit_event_id,command_vault_path FROM evidence_verification_command_results WHERE operation='update_evidence_verification' AND idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let evidence_id = EvidenceReferenceId::parse(row.0).map_err(|_| storage_error(context))?;
    let immutable = load_evidence_reference_immutable_fields(tx, &evidence_id, context)?;
    let audit_event = decode_audit(tx, &row.5, context)?;
    let record = EvidenceReferenceRecord {
        id: evidence_id,
        // The path this verification was recorded against, not the
        // live row's, which a later relocation may have moved.
        vault_path: VaultRelativePath::parse(row.6).map_err(|_| storage_error(context))?,
        fingerprint: immutable.fingerprint,
        verification: decode_verification(&row.1, row.2, row.3, context)?,
        classification: immutable.classification,
        provenance: immutable.provenance,
        version: AggregateVersion::new(u64::try_from(row.4).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
        created_at: immutable.created_at,
        updated_at: audit_event.occurred_at(),
    };
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

/// Decode the exact outcome `relocate_evidence_reference` produced for a
/// given idempotency key. `vault_path` is reconstructed from that command's
/// own stored `command_new_vault_path` -- NOT the current row's path, which
/// a later relocation could have moved again. Since v45
/// [`decode_evidence_verification_outcome`] reconstructs its path the same
/// way from `command_vault_path`; before v45 it read the live row.
/// Staleness aside, Only the fields this command never touches (fingerprint/
/// classification/provenance/created_at) are re-read from current state.
fn decode_evidence_relocation_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<MutationOutcome<EvidenceReferenceRecord>, DomainError> {
    let row = tx
        .query_row(
            "SELECT command_id,command_new_vault_path,command_observed_fingerprint_digest,command_observed_at,result_version,audit_event_id FROM evidence_relocation_command_results WHERE operation='relocate_evidence_reference' AND idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let evidence_id = EvidenceReferenceId::parse(row.0).map_err(|_| storage_error(context))?;
    let immutable = load_evidence_reference_immutable_fields(tx, &evidence_id, context)?;
    let audit_event = decode_audit(tx, &row.5, context)?;
    let record = EvidenceReferenceRecord {
        id: evidence_id,
        vault_path: VaultRelativePath::parse(row.1).map_err(|_| storage_error(context))?,
        fingerprint: immutable.fingerprint,
        verification: EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(row.3),
            integrity_digest: IntegrityDigest::parse(row.2).map_err(|_| storage_error(context))?,
        },
        classification: immutable.classification,
        provenance: immutable.provenance,
        version: AggregateVersion::new(u64::try_from(row.4).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
        created_at: immutable.created_at,
        updated_at: audit_event.occurred_at(),
    };
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

/// Decode the exact outcome `pin_evidence_fingerprint` produced for a given
/// idempotency key. Path, fingerprint, verification and version are rebuilt
/// from the command's own stored fields -- NOT the live row, which a later
/// relocation may have moved -- following [`decode_evidence_relocation_outcome`].
/// Only the fields this command never touches (classification/provenance/
/// created_at) are re-read from current state.
fn decode_evidence_fingerprint_pin_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<MutationOutcome<EvidenceReferenceRecord>, DomainError> {
    let row = tx
        .query_row(
            "SELECT command_id,command_expected_current_path,command_observed_fingerprint_algorithm,command_observed_fingerprint_digest,command_observed_at,result_version,audit_event_id FROM evidence_fingerprint_pin_command_results WHERE operation='pin_evidence_fingerprint' AND idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let evidence_id = EvidenceReferenceId::parse(row.0).map_err(|_| storage_error(context))?;
    let immutable = load_evidence_reference_immutable_fields(tx, &evidence_id, context)?;
    let audit_event = decode_audit(tx, &row.6, context)?;
    let algorithm =
        FingerprintAlgorithm::from_persisted(&row.2).map_err(|_| storage_error(context))?;
    let digest = IntegrityDigest::parse(row.3).map_err(|_| storage_error(context))?;
    let record = EvidenceReferenceRecord {
        id: evidence_id,
        vault_path: VaultRelativePath::parse(row.1).map_err(|_| storage_error(context))?,
        fingerprint: Some(EvidenceFingerprint::new(algorithm, digest.clone())),
        verification: EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(row.4),
            integrity_digest: digest,
        },
        classification: immutable.classification,
        provenance: immutable.provenance,
        version: AggregateVersion::new(u64::try_from(row.5).map_err(|_| storage_error(context))?)
            .map_err(|_| storage_error(context))?,
        created_at: immutable.created_at,
        updated_at: audit_event.occurred_at(),
    };
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

fn verification_kind(verification: &EvidenceVerification) -> &'static str {
    match verification {
        EvidenceVerification::Verified { .. } => "verified",
        EvidenceVerification::ObservedUnpinned { .. } => "observed_unpinned",
        EvidenceVerification::DegradedLastVerified { .. } => "degraded_last_verified",
        EvidenceVerification::Unverified => "unverified",
        EvidenceVerification::IntegrityMismatch => "integrity_mismatch",
    }
}

fn verification_last_verified_at(verification: &EvidenceVerification) -> Option<i64> {
    match verification {
        EvidenceVerification::Verified { verified_at, .. } => Some(verified_at.unix_millis()),
        EvidenceVerification::ObservedUnpinned { observed_at, .. } => {
            Some(observed_at.unix_millis())
        }
        EvidenceVerification::DegradedLastVerified {
            last_verified_at, ..
        } => Some(last_verified_at.unix_millis()),
        EvidenceVerification::Unverified | EvidenceVerification::IntegrityMismatch => None,
    }
}

fn verification_integrity_digest(verification: &EvidenceVerification) -> Option<&str> {
    match verification {
        EvidenceVerification::Verified {
            integrity_digest, ..
        }
        | EvidenceVerification::DegradedLastVerified {
            integrity_digest, ..
        }
        | EvidenceVerification::ObservedUnpinned {
            integrity_digest, ..
        } => Some(integrity_digest.as_str()),
        EvidenceVerification::Unverified | EvidenceVerification::IntegrityMismatch => None,
    }
}

/// Thin wrapper over the domain's one decoder, keeping the write path's
/// storage-error mapping. The three-column mapping itself lives in
/// `EvidenceVerification::from_persisted_parts` so the composition read
/// surface and this path cannot drift apart.
fn decode_verification(
    kind: &str,
    last_verified_at: Option<i64>,
    integrity_digest: Option<String>,
    context: &OperationContext,
) -> Result<EvidenceVerification, DomainError> {
    let digest = integrity_digest
        .map(IntegrityDigest::parse)
        .transpose()
        .map_err(|_| storage_error(context))?;
    EvidenceVerification::from_persisted_parts(
        kind,
        last_verified_at.map(UtcTimestamp::from_unix_millis),
        digest,
    )
    .map_err(|_| storage_error(context))
}

fn decode_provenance(
    kind: &str,
    reference: Option<String>,
    context: &OperationContext,
) -> Result<Provenance, DomainError> {
    match kind {
        "user_entered" => Ok(Provenance::UserEntered),
        "authoritative_transition" => Ok(Provenance::AuthoritativeTransition(
            ProvenanceReference::parse(reference.ok_or_else(|| storage_error(context))?)
                .map_err(|_| storage_error(context))?,
        )),
        "synthetic_fixture" => Ok(Provenance::SyntheticFixture(
            ProvenanceReference::parse(reference.ok_or_else(|| storage_error(context))?)
                .map_err(|_| storage_error(context))?,
        )),
        _ => Err(storage_error(context)),
    }
}

fn decode_evidence_reference_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<MutationOutcome<EvidenceReferenceRecord>, DomainError> {
    let row = tx
        .query_row(
            "SELECT command_id,command_vault_relative_path,command_fingerprint_algorithm,command_fingerprint_digest,command_verification,command_last_verified_at,command_integrity_digest,result_classification,command_provenance_kind,command_provenance_reference,result_created_at,audit_event_id FROM evidence_reference_command_results WHERE operation='create_evidence_reference' AND idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, String>(11)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let fingerprint = match (row.2, row.3) {
        (Some(algorithm), Some(digest)) => Some(EvidenceFingerprint::new(
            FingerprintAlgorithm::from_persisted(&algorithm).map_err(|_| storage_error(context))?,
            IntegrityDigest::parse(digest).map_err(|_| storage_error(context))?,
        )),
        (None, None) => None,
        _ => return Err(storage_error(context)),
    };
    let occurred_at = UtcTimestamp::from_unix_millis(row.10);
    let record = EvidenceReferenceRecord {
        id: EvidenceReferenceId::parse(row.0).map_err(|_| storage_error(context))?,
        vault_path: VaultRelativePath::parse(row.1).map_err(|_| storage_error(context))?,
        fingerprint,
        verification: decode_verification(&row.4, row.5, row.6, context)?,
        classification: DataClassification::from_persisted(&row.7)
            .map_err(|_| storage_error(context))?,
        provenance: decode_provenance(&row.8, row.9, context)?,
        version: AggregateVersion::initial(),
        created_at: occurred_at,
        updated_at: occurred_at,
    };
    let audit_event = decode_audit(tx, &row.11, context)?;
    Ok(MutationOutcome {
        record,
        audit_event,
    })
}

fn decode_audit(
    tx: &Transaction<'_>,
    audit_event_id: &str,
    context: &OperationContext,
) -> Result<AuditEvent, DomainError> {
    let row = tx
        .query_row(
            "SELECT occurred_at,event_code,target_id,correlation_id FROM audit_events WHERE id=?1",
            [audit_event_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    build_audit(
        AuditEventId::parse(audit_event_id).map_err(|_| storage_error(context))?,
        UtcTimestamp::from_unix_millis(row.0),
        AuditTarget::EvidenceReference(
            EvidenceReferenceId::parse(row.2).map_err(|_| storage_error(context))?,
        ),
        &row.1,
        &OperationContext {
            idempotency_id: context.idempotency_id.clone(),
            correlation_id: CorrelationId::parse(row.3).map_err(|_| storage_error(context))?,
        },
    )
}

fn build_audit(
    id: AuditEventId,
    at: UtcTimestamp,
    target: AuditTarget,
    code: &str,
    context: &OperationContext,
) -> Result<AuditEvent, DomainError> {
    Ok(AuditEvent::new(
        id,
        at,
        AuditActor::HeadOfProducts,
        AuditAction::new(
            AuditModule::WorkManagement,
            AuditEventCode::parse(code).map_err(|_| storage_error(context))?,
            target,
        ),
        context.correlation_id.clone(),
        AuditDisposition::new(
            AuditPolicyOutcome::NotRequired,
            pmc_domain::audit::AuditApprovalOutcome::NotRequired,
            AuditExecutionOutcome::Succeeded,
            AuditEffectScope::Complete,
            vec![AuditEffectCode::parse(EVIDENCE_EFFECT_CODE).map_err(|_| storage_error(context))?],
        )
        .map_err(|_| storage_error(context))?,
    ))
}

fn persist_evidence_audit(
    tx: &Transaction<'_>,
    audit: &AuditEvent,
    target_id: &str,
    context: &OperationContext,
) -> Result<(), DomainError> {
    tx.execute(
        "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,'head_of_products','work_management',?3,'evidence_reference',?4,?5,'not_required','not_required','succeeded','complete')",
        rusqlite::params![
            audit.id().as_str(),
            audit.occurred_at().unix_millis(),
            audit.code().as_str(),
            target_id,
            context.correlation_id.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,0,?2,'complete','evidence_reference',?3)",
        rusqlite::params![audit.id().as_str(), EVIDENCE_EFFECT_CODE, target_id],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

fn next_evidence_operation_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,1) FROM evidence_reference_command_results",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

/// The create transaction shared by [`SqliteProductLedger::create_evidence_reference`]
/// and the reserved create (v48): replay, then — for Evidence from a file —
/// the one-reference-per-Vault-file check, then the insert.
fn insert_evidence_reference(
    tx: &mut Transaction<'_>,
    command: CreateEvidenceReference,
    audit_event_id: AuditEventId,
    occurred_at: UtcTimestamp,
    expected_revision: u64,
    same_name: Option<fn(&str, &str) -> bool>,
) -> Result<MutationOutcome<EvidenceReferenceRecord>, DomainError> {
    let context = command.context.clone();
    if let Some(existing) = tx
            .query_row(
                "SELECT command_id,command_vault_relative_path,command_fingerprint_algorithm,command_fingerprint_digest,command_verification,command_last_verified_at,command_integrity_digest,command_classification,command_provenance_kind,command_provenance_reference FROM evidence_reference_command_results WHERE operation='create_evidence_reference' AND idempotency_id=?1",
                [context.idempotency_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, Option<String>>(9)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| storage_error(&context))?
        {
            let matches = existing.0 == command.id.as_str()
                && existing.1 == command.vault_path.as_str()
                && existing.2.as_deref()
                    == command
                        .fingerprint
                        .as_ref()
                        .map(|value| value.algorithm().as_persisted())
                && existing.3.as_deref()
                    == command.fingerprint.as_ref().map(|value| value.digest().as_str())
                && existing.4 == verification_kind(&command.verification)
                && existing.5 == verification_last_verified_at(&command.verification)
                && existing.6.as_deref() == verification_integrity_digest(&command.verification)
                && existing.7.as_deref()
                    == command.classification.map(DataClassification::as_persisted)
                && existing.8 == command.provenance.kind_persisted()
                && existing.9.as_deref()
                    == command.provenance.reference().map(ProvenanceReference::as_str);
            if matches {
                return decode_evidence_reference_outcome(tx, &context);
            }
            return Err(idempotency_conflict(&context));
        }
    // Evidence from a file (v48): one reference per Vault file. Checked
    // here, under the write lock, after the replay above (a replay of
    // this very create finds its own path) and before anything is
    // written; the host's earlier read is guidance, not this check.
    if let Some(same_name) = same_name {
        if vault_path_taken(tx, command.vault_path.as_str(), same_name)
            .map_err(|_| storage_error(&context))?
        {
            return Err(path_already_referenced(&context));
        }
    }
    let exists: i64 = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM aggregate_registry WHERE id=?1)",
            [command.id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(&context))?;
    if exists != 0 {
        return Err(domain_conflict(&context));
    }
    let classification = command.classification.unwrap_or_default();
    let occurred_millis = occurred_at.unix_millis();
    tx.execute(
            "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES(?1,'evidence_reference',1,?2,?3,?3)",
            rusqlite::params![command.id.as_str(), classification.as_persisted(), occurred_millis],
        )
        .map_err(|_| storage_error(&context))?;
    tx.execute(
            "INSERT INTO evidence_references(id,role,verification,integrity_digest,last_verified_at,vault_relative_path,fingerprint_algorithm,fingerprint_digest,provenance_kind,provenance_reference) VALUES(?1,NULL,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![
                command.id.as_str(),
                verification_kind(&command.verification),
                verification_integrity_digest(&command.verification),
                verification_last_verified_at(&command.verification),
                command.vault_path.as_str(),
                command.fingerprint.as_ref().map(|value| value.algorithm().as_persisted()),
                command.fingerprint.as_ref().map(|value| value.digest().as_str()),
                command.provenance.kind_persisted(),
                command.provenance.reference().map(ProvenanceReference::as_str),
            ],
        )
        .map_err(|_| storage_error(&context))?;
    let audit = build_audit(
        audit_event_id,
        occurred_at,
        AuditTarget::EvidenceReference(command.id.clone()),
        "evidence.reference_created",
        &context,
    )?;
    persist_evidence_audit(tx, &audit, command.id.as_str(), &context)?;
    let ordinal = next_evidence_operation_ordinal(tx, &context)?;
    tx.execute(
            "INSERT INTO evidence_reference_command_results (operation,idempotency_id,correlation_id,operation_ordinal,command_id,command_vault_relative_path,command_fingerprint_algorithm,command_fingerprint_digest,command_verification,command_last_verified_at,command_integrity_digest,command_classification,command_provenance_kind,command_provenance_reference,result_classification,result_created_at,audit_event_id) VALUES ('create_evidence_reference',?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            rusqlite::params![
                context.idempotency_id.as_str(),
                context.correlation_id.as_str(),
                ordinal,
                command.id.as_str(),
                command.vault_path.as_str(),
                command.fingerprint.as_ref().map(|value| value.algorithm().as_persisted()),
                command.fingerprint.as_ref().map(|value| value.digest().as_str()),
                verification_kind(&command.verification),
                verification_last_verified_at(&command.verification),
                verification_integrity_digest(&command.verification),
                command.classification.map(DataClassification::as_persisted),
                command.provenance.kind_persisted(),
                command.provenance.reference().map(ProvenanceReference::as_str),
                classification.as_persisted(),
                occurred_millis,
                audit.id().as_str(),
            ],
        )
        .map_err(|_| storage_error(&context))?;
    let expected_revision =
        i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
    if tx
            .execute(
                "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
                rusqlite::params![expected_revision + 1, expected_revision],
            )
            .map_err(|_| storage_error(&context))?
            != 1
        {
            return Err(storage_error(&context));
        }
    Ok(MutationOutcome {
        record: EvidenceReferenceRecord {
            id: command.id,
            vault_path: command.vault_path,
            fingerprint: command.fingerprint,
            verification: command.verification,
            classification,
            provenance: command.provenance,
            version: AggregateVersion::initial(),
            created_at: occurred_at,
            updated_at: occurred_at,
        },
        audit_event: audit,
    })
}

fn evidence_match(id: String, version: i64) -> Result<EvidenceMatch, LedgerOpenError> {
    Ok(EvidenceMatch {
        id: EvidenceReferenceId::parse(id).map_err(|_| LedgerOpenError::StorageUnavailable)?,
        version: u64::try_from(version)
            .ok()
            .and_then(|value| AggregateVersion::new(value).ok())
            .ok_or(LedgerOpenError::StorageUnavailable)?,
    })
}

/// Inside a create transaction: whether any reference already names `path`
/// — the exact spelling through its index first, then every stored path
/// compared with `same_name`.
fn vault_path_taken(
    connection: &rusqlite::Connection,
    path: &str,
    same_name: fn(&str, &str) -> bool,
) -> Result<bool, rusqlite::Error> {
    let exact: i64 = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM evidence_references WHERE vault_relative_path=?1)",
        [path],
        |row| row.get(0),
    )?;
    if exact != 0 {
        return Ok(true);
    }
    let mut statement =
        connection.prepare("SELECT vault_relative_path FROM evidence_references")?;
    let paths = statement.query_map([], |row| row.get::<_, String>(0))?;
    for stored in paths {
        if same_name(&stored?, path) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn path_already_referenced(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("evidence.path_already_referenced").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn storage_error(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::PlatformInternal,
        MessageKey::parse("evidence.persistence_failed").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        true,
    )
}

fn idempotency_conflict(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainIdempotencyConflict,
        MessageKey::parse("evidence.idempotency_conflict").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn domain_conflict(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("evidence.already_exists").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn domain_not_found(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainNotFound,
        MessageKey::parse("evidence.not_found").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn relocation_path_unchanged(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("evidence.relocation_path_unchanged").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn relocation_path_mismatch(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("evidence.relocation_path_mismatch").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn relocation_fingerprint_unpinned(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("evidence.relocation_fingerprint_unpinned")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn relocation_fingerprint_mismatch(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("evidence.relocation_fingerprint_mismatch")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn pin_path_mismatch(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("evidence.pin_path_mismatch").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn pin_fingerprint_already_pinned(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("evidence.pin_fingerprint_already_pinned")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}
