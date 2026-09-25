//! Typed SQLite persistence seam for the Relationship family's H2b guarded
//! removal flow: `PrepareRemoveRelationship` / `CancelRemoveRelationship` /
//! `ApproveAndExecuteRemoveRelationship`. Shares its helper functions with
//! `relationship_repository.rs` (the ordinary H1 commands) via `pub(super)`.
//!
//! The prepared-intent preview and its cryptographic payload digest are
//! built through `pmc_domain::execution`'s own public, `#[doc(hidden)]`
//! persistence-adapter constructors (`RemoveRelationshipPreview::from_persistence`,
//! `PreparedIntent::from_persistence_preview`) rather than reimplemented here:
//! `PayloadDigest::for_preview`/`for_confirmation` are `pub(crate)` to
//! `pmc-domain` and therefore unreachable from this crate directly, and
//! hand-rolling an equivalent hash would risk silently drifting from the
//! domain's own algorithm. This writer computes state changes and existence
//! checks itself in SQL, matching the rest of this crate's writers, and
//! delegates only the digest-sensitive reconstruction to the domain.

use pmc_domain::{
    audit::{
        AuditActor, AuditApprovalOutcome, AuditDisposition, AuditEffectScope, AuditEventCode,
        AuditExecutionOutcome, AuditModule, AuditPolicyOutcome, AuditTarget,
    },
    classification::DataClassification,
    error::{DomainError, ErrorCode, MessageKey},
    execution::{
        ApprovalAuthorizationPort, ApproveAndExecuteRemoveRelationship, CancellationPolicy,
        PrepareRemoveRelationship, PreparedIntent, RecoveryEvidence, RecoveryEvidencePort,
        RemovalEffect, RemovalOutcome, RemovalPolicyDecision, RemovalPolicyPort,
        RemoveRelationshipPreview,
    },
    identity::{AggregateVersion, AuditEventId, PreparedIntentId, RelationshipId},
    relationships::{OperationContext, RelationshipKind, StakeholderRelationshipPurpose},
    time::UtcTimestamp,
};
use rusqlite::{OptionalExtension, Transaction};

use super::relationship_repository::{
    build_endpoint_snapshot, domain_conflict, domain_not_found, idempotency_conflict,
    link_type_to_aggregate_type, next_relationship_operation_ordinal, policy_denied, storage_error,
};
use super::{LedgerTransactionError, SqliteProductLedger};

impl SqliteProductLedger {
    pub fn prepare_remove_relationship<PV: RecoveryEvidencePort, RP: RemovalPolicyPort>(
        &mut self,
        command: PrepareRemoveRelationship,
        recovery: &PV,
        removal_policy: &RP,
        prepared_intent_id: PreparedIntentId,
        occurred_at: UtcTimestamp,
    ) -> Result<PreparedIntent, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing_relationship_id) = tx
                .query_row(
                    "SELECT relationship_id FROM relationship_h2b_command_results WHERE idempotency_id=?1 AND command_kind='prepare_remove'",
                    [context.idempotency_id.as_str()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if existing_relationship_id == command.relationship_id.as_str() {
                    return decode_prepared_intent_outcome(tx, "prepare_remove", &context);
                }
                return Err(idempotency_conflict(&context));
            }

            let (kind_persisted, purpose_persisted, relationship_classification, relationship_version): (
                String,
                Option<String>,
                String,
                i64,
            ) = tx
                .query_row(
                    "SELECT relationships.kind,relationships.purpose,registry.classification,registry.version FROM relationships JOIN aggregate_registry registry ON registry.id=relationships.id AND registry.aggregate_type='relationship' WHERE relationships.id=?1",
                    [command.relationship_id.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context))?;
            let kind = RelationshipKind::from_persisted(&kind_persisted)
                .map_err(|_| storage_error(&context))?;
            let purpose = purpose_persisted
                .map(|value| {
                    StakeholderRelationshipPurpose::from_persisted(&value)
                        .map_err(|_| storage_error(&context))
                })
                .transpose()?;

            let endpoint_identities: Vec<(String, String)> = {
                let mut statement = tx
                    .prepare("SELECT target_type,target_id FROM relationship_endpoints WHERE relationship_id=?1 ORDER BY ordinal")
                    .map_err(|_| storage_error(&context))?;
                let rows = statement
                    .query_map([command.relationship_id.as_str()], |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })
                    .map_err(|_| storage_error(&context))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| storage_error(&context))?;
                rows
            };
            let mut endpoints = Vec::with_capacity(endpoint_identities.len());
            let mut classification = DataClassification::from_persisted(&relationship_classification)
                .map_err(|_| storage_error(&context))?;
            for (target_type, target_id) in &endpoint_identities {
                let (version, endpoint_classification): (i64, String) = tx
                    .query_row(
                        "SELECT version,classification FROM aggregate_registry WHERE id=?1 AND aggregate_type=?2",
                        [target_id.as_str(), target_type.as_str()],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map_err(|_| storage_error(&context))?;
                let endpoint_classification = DataClassification::from_persisted(&endpoint_classification)
                    .map_err(|_| storage_error(&context))?;
                classification = classification.combine(endpoint_classification);
                let version = AggregateVersion::new(u64::try_from(version).map_err(|_| storage_error(&context))?)
                    .map_err(|_| storage_error(&context))?;
                endpoints.push(build_endpoint_snapshot(
                    target_type,
                    target_id,
                    version,
                    endpoint_classification,
                    &context,
                )?);
            }

            if !removal_policy.allow_relationship_removal(&command.relationship_id, classification) {
                return Err(policy_denied(&context));
            }
            let evidence = recovery
                .recovery_evidence(&command.relationship_id)
                .filter(|value| {
                    value.relationship_id() == &command.relationship_id && value.compatible()
                })
                .filter(|value| value.verified_at().unix_millis() > 0);
            let Some(evidence) = evidence else {
                return Err(policy_denied(&context));
            };
            let occurred_millis = occurred_at.unix_millis();
            let expires_at_millis = occurred_millis
                .checked_add(pmc_domain::execution::MAX_PREPARED_INTENT_TTL_MILLIS)
                .ok_or_else(|| storage_error(&context))?;
            if classification == DataClassification::Unclassified || expires_at_millis <= occurred_millis
            {
                return Err(policy_denied(&context));
            }

            let exists: i64 = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM prepared_intents WHERE id=?1)",
                    [prepared_intent_id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| storage_error(&context))?;
            if exists != 0 {
                return Err(domain_conflict(&context));
            }
            let outstanding: i64 = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM prepared_removal_payloads payload JOIN prepared_intents intent ON intent.id=payload.prepared_intent_id WHERE payload.relationship_id=?1 AND intent.consumed_at IS NULL AND NOT EXISTS(SELECT 1 FROM relationship_h2b_command_results WHERE command_kind='cancel_remove' AND prepared_intent_id=intent.id))",
                    [command.relationship_id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| storage_error(&context))?;
            if outstanding != 0 {
                return Err(domain_conflict(&context));
            }

            let confirmation_challenge = format!("REMOVE {prepared_intent_id}");
            let preview = RemoveRelationshipPreview::from_persistence(
                prepared_intent_id.clone(),
                command.relationship_id.clone(),
                AggregateVersion::new(u64::try_from(relationship_version).map_err(|_| storage_error(&context))?)
                    .map_err(|_| storage_error(&context))?,
                endpoints,
                kind,
                purpose,
                vec![
                    RemovalEffect::RemoveRelationshipRecord,
                    RemovalEffect::RemoveSemanticRelationshipIndex,
                    RemovalEffect::CreateIdempotencyTombstone,
                ],
                classification,
                RemovalPolicyDecision::Allowed,
                evidence,
                UtcTimestamp::from_unix_millis(expires_at_millis),
                CancellationPolicy::NotCancellableAfterSubmit,
                confirmation_challenge,
            );
            let prepared = PreparedIntent::from_persistence_preview(preview);

            persist_prepared_intent(tx, &prepared, occurred_millis, &context)?;
            let ordinal = next_relationship_operation_ordinal(tx, &context)?;
            tx.execute(
                "INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES(?1,'prepare_remove',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO relationship_h2b_command_results(idempotency_id,command_kind,result_kind,relationship_id,result_prepared_intent_id) VALUES(?1,'prepare_remove','prepared',?2,?3)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.relationship_id.as_str(),
                    prepared.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;

            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
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

    pub fn cancel_remove_relationship<AP: ApprovalAuthorizationPort>(
        &mut self,
        prepared_intent_id: PreparedIntentId,
        actor: AuditActor,
        context: OperationContext,
        approval_authorization: &AP,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<(), LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&context)))?;
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if actor != AuditActor::HeadOfProducts
                || !approval_authorization.authorize_relationship_removal(actor)
            {
                return Err(policy_denied(&context));
            }
            if let Some((stored_prepared_id, stored_actor)) = tx
                .query_row(
                    "SELECT prepared_intent_id,actor FROM relationship_h2b_command_results WHERE idempotency_id=?1 AND command_kind='cancel_remove'",
                    [context.idempotency_id.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                if stored_prepared_id == prepared_intent_id.as_str()
                    && stored_actor == "head_of_products"
                    && actor == AuditActor::HeadOfProducts
                {
                    return Ok(());
                }
                return Err(idempotency_conflict(&context));
            }
            let consumed_at: Option<i64> = tx
                .query_row(
                    "SELECT consumed_at FROM prepared_intents WHERE id=?1",
                    [prepared_intent_id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| domain_not_found(&context))?;
            if consumed_at.is_some() {
                return Err(too_late_to_cancel(&context));
            }
            let already_cancelled: i64 = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM relationship_h2b_command_results WHERE command_kind='cancel_remove' AND prepared_intent_id=?1)",
                    [prepared_intent_id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| storage_error(&context))?;
            if already_cancelled != 0 {
                return Err(domain_not_found(&context));
            }
            let relationship_id: String = tx
                .query_row(
                    "SELECT relationship_id FROM prepared_removal_payloads WHERE prepared_intent_id=?1",
                    [prepared_intent_id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| storage_error(&context))?;

            let audit = build_execution_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Relationship(
                    RelationshipId::parse(relationship_id).map_err(|_| storage_error(&context))?,
                ),
                "relationship.removal.cancelled_before_approval",
                actor,
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::NotRequired,
                AuditExecutionOutcome::Cancelled,
                AuditEffectScope::None,
                Vec::new(),
                &context,
            )?;
            persist_execution_audit(tx, &audit, &context)?;
            let ordinal = next_relationship_operation_ordinal(tx, &context)?;
            tx.execute(
                "INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES(?1,'cancel_remove',?2,?3,'cancelled',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared_intent_id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO relationship_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
                rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO relationship_h2b_command_results(idempotency_id,command_kind,result_kind,prepared_intent_id,actor) VALUES(?1,'cancel_remove','cancelled',?2,'head_of_products')",
                rusqlite::params![context.idempotency_id.as_str(), prepared_intent_id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;

            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
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
            Ok(())
        })
    }

    /// The domain's own `approve_and_execute_remove_relationship` mints an
    /// `EphemeralApprovalReceipt` and validates it just before committing;
    /// that check exists to guard the in-memory service's own atomicity
    /// discipline ("bind and consume inside this one state transition") and
    /// is unreachable here once every earlier check in this method has
    /// passed, since this method's own SQL transaction is what provides
    /// atomicity -- so it is deliberately not reimplemented.
    #[allow(clippy::too_many_lines)]
    pub fn approve_and_execute_remove_relationship<
        PV: RecoveryEvidencePort,
        RP: RemovalPolicyPort,
        AP: ApprovalAuthorizationPort,
    >(
        &mut self,
        command: ApproveAndExecuteRemoveRelationship,
        recovery: &PV,
        removal_policy: &RP,
        approval_authorization: &AP,
        audit_event_id: AuditEventId,
        occurred_at: UtcTimestamp,
    ) -> Result<RemovalOutcome, LedgerTransactionError<DomainError>> {
        let expected_revision = self
            .revision()
            .map_err(|_| LedgerTransactionError::Operation(storage_error(&command.context)))?;
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            let confirmation_digest = confirmation_digest_for(&command.confirmation);

            if let Some((stored_prepared_id, stored_actor, stored_ack_digest, stored_confirmation_digest)) = tx
                .query_row(
                    "SELECT prepared_intent_id,actor,acknowledged_payload_digest,confirmation_digest FROM relationship_h2b_command_results WHERE idempotency_id=?1 AND command_kind='execute_remove'",
                    [context.idempotency_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| storage_error(&context))?
            {
                let matches = stored_prepared_id == command.prepared_id.as_str()
                    && stored_actor == "head_of_products"
                    && command.actor == AuditActor::HeadOfProducts
                    && stored_ack_digest == command.acknowledged_payload_digest.as_str()
                    && stored_confirmation_digest == confirmation_digest.as_str();
                if matches {
                    return decode_execute_outcome(tx, &context);
                }
                return Err(idempotency_conflict(&context));
            }

            let known_relationship_id: Option<String> = tx
                .query_row(
                    "SELECT relationship_id FROM prepared_removal_payloads WHERE prepared_intent_id=?1",
                    [command.prepared_id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| storage_error(&context))?;

            macro_rules! reject {
                ($code:expr, $policy:expr, $approval:expr, $execution:expr, $error:expr) => {{
                    let Some(relationship_id) = &known_relationship_id else {
                        return Err($error);
                    };
                    return reject_execute(
                        tx,
                        &context,
                        RelationshipId::parse(relationship_id.as_str()).map_err(|_| storage_error(&context))?,
                        command.actor,
                        $code,
                        $policy,
                        $approval,
                        $execution,
                        $error,
                        &command,
                        &confirmation_digest,
                        audit_event_id,
                        occurred_at,
                        expected_revision,
                    );
                }};
            }

            if command.actor != AuditActor::HeadOfProducts
                || !approval_authorization.authorize_relationship_removal(command.actor)
            {
                reject!(
                    "relationship.removal.approval_rejected",
                    AuditPolicyOutcome::Allowed,
                    AuditApprovalOutcome::Rejected,
                    AuditExecutionOutcome::NotAttempted,
                    authorization_denied(&context)
                );
            }

            let consumed_at: Option<i64> = tx
                .query_row(
                    "SELECT consumed_at FROM prepared_intents WHERE id=?1",
                    [command.prepared_id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .flatten();
            let intent_exists = known_relationship_id.is_some();
            if !intent_exists {
                reject!(
                    "relationship.removal.approval_rejected",
                    AuditPolicyOutcome::Allowed,
                    AuditApprovalOutcome::Rejected,
                    AuditExecutionOutcome::NotAttempted,
                    domain_not_found(&context)
                );
            }
            // A cancelled prepared intent is simply gone from the domain's
            // outstanding-prepares map (not tracked as "completed"), so the
            // domain's own not-found branch is what fires for it -- not a
            // conflict, even though the row itself still durably exists here
            // for history.
            let cancelled: i64 = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM relationship_h2b_command_results WHERE command_kind='cancel_remove' AND prepared_intent_id=?1)",
                    [command.prepared_id.as_str()],
                    |row| row.get(0),
                )
                .map_err(|_| storage_error(&context))?;
            if cancelled != 0 {
                reject!(
                    "relationship.removal.approval_rejected",
                    AuditPolicyOutcome::Allowed,
                    AuditApprovalOutcome::Rejected,
                    AuditExecutionOutcome::NotAttempted,
                    domain_not_found(&context)
                );
            }
            if consumed_at.is_some() {
                reject!(
                    "relationship.removal.approval_rejected",
                    AuditPolicyOutcome::Allowed,
                    AuditApprovalOutcome::Rejected,
                    AuditExecutionOutcome::NotAttempted,
                    domain_conflict(&context)
                );
            }

            let prepared = decode_prepared_intent(tx, command.prepared_id.as_str(), &context)?;
            let preview = prepared.preview();

            if preview.confirmation_challenge() != command.confirmation {
                reject!(
                    "relationship.removal.approval_rejected",
                    AuditPolicyOutcome::Allowed,
                    AuditApprovalOutcome::Rejected,
                    AuditExecutionOutcome::NotAttempted,
                    confirmation_mismatch(&context)
                );
            }
            if preview.expires_at().unix_millis() <= occurred_at.unix_millis()
                || prepared.payload_digest().as_str() != command.acknowledged_payload_digest.as_str()
            {
                reject!(
                    "relationship.removal.approval_rejected",
                    AuditPolicyOutcome::Allowed,
                    AuditApprovalOutcome::Rejected,
                    AuditExecutionOutcome::NotAttempted,
                    preview_expired_or_changed(&context)
                );
            }

            let current_row: Option<(String, Option<String>, String, i64)> = tx
                .query_row(
                    "SELECT relationships.kind,relationships.purpose,registry.classification,registry.version FROM relationships JOIN aggregate_registry registry ON registry.id=relationships.id AND registry.aggregate_type='relationship' WHERE relationships.id=?1",
                    [preview.relationship_id().as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()
                .map_err(|_| storage_error(&context))?;
            let Some((current_kind_persisted, current_purpose_persisted, current_relationship_classification, current_relationship_version)) = current_row else {
                reject!(
                    "relationship.removal.execution_rejected",
                    AuditPolicyOutcome::Allowed,
                    AuditApprovalOutcome::Approved,
                    AuditExecutionOutcome::Failed,
                    preview_expired_or_changed(&context)
                );
            };
            let current_kind = RelationshipKind::from_persisted(&current_kind_persisted).map_err(|_| storage_error(&context))?;
            let current_purpose = current_purpose_persisted
                .map(|value| StakeholderRelationshipPurpose::from_persisted(&value).map_err(|_| storage_error(&context)))
                .transpose()?;

            let endpoint_identities: Vec<(String, String)> = {
                let mut statement = tx
                    .prepare("SELECT target_type,target_id FROM relationship_endpoints WHERE relationship_id=?1 ORDER BY ordinal")
                    .map_err(|_| storage_error(&context))?;
                let rows = statement
                    .query_map([preview.relationship_id().as_str()], |row| Ok((row.get(0)?, row.get(1)?)))
                    .map_err(|_| storage_error(&context))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| storage_error(&context))?;
                rows
            };
            let mut current_endpoints = Vec::with_capacity(endpoint_identities.len());
            let mut current_classification = DataClassification::from_persisted(&current_relationship_classification)
                .map_err(|_| storage_error(&context))?;
            let mut resolution_failed = false;
            for (target_type, target_id) in &endpoint_identities {
                let resolved: Option<(i64, String)> = tx
                    .query_row(
                        "SELECT version,classification FROM aggregate_registry WHERE id=?1 AND aggregate_type=?2",
                        [target_id.as_str(), target_type.as_str()],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
                    .map_err(|_| storage_error(&context))?;
                let Some((version, classification)) = resolved else {
                    resolution_failed = true;
                    break;
                };
                let classification = DataClassification::from_persisted(&classification).map_err(|_| storage_error(&context))?;
                current_classification = current_classification.combine(classification);
                let version = AggregateVersion::new(u64::try_from(version).map_err(|_| storage_error(&context))?)
                    .map_err(|_| storage_error(&context))?;
                current_endpoints.push(build_endpoint_snapshot(target_type, target_id, version, classification, &context)?);
            }
            if resolution_failed {
                reject!(
                    "relationship.removal.execution_rejected",
                    AuditPolicyOutcome::Allowed,
                    AuditApprovalOutcome::Approved,
                    AuditExecutionOutcome::Failed,
                    preview_expired_or_changed(&context)
                );
            }

            let current_evidence = recovery.recovery_evidence(preview.relationship_id());
            if current_classification == DataClassification::Unclassified
                || !removal_policy.allow_relationship_removal(preview.relationship_id(), current_classification)
                || !current_evidence.as_ref().is_some_and(|value| {
                    value.relationship_id() == preview.relationship_id()
                        && value.compatible()
                        && value.verified_at().unix_millis() > 0
                })
            {
                reject!(
                    "relationship.removal.policy_denied",
                    AuditPolicyOutcome::Denied,
                    AuditApprovalOutcome::NotRequired,
                    AuditExecutionOutcome::NotAttempted,
                    policy_denied(&context)
                );
            }
            let evidence_matches_prepared = current_evidence
                .as_ref()
                .is_some_and(|value| value == preview.evidence());
            let current_versions: Vec<AggregateVersion> = current_endpoints
                .iter()
                .map(super::relationship_repository::endpoint_snapshot_version)
                .collect();
            let prepared_versions: Vec<AggregateVersion> = preview
                .endpoints()
                .iter()
                .map(super::relationship_repository::endpoint_snapshot_version)
                .collect();
            if AggregateVersion::new(u64::try_from(current_relationship_version).unwrap_or(0)) != Ok(preview.relationship_version())
                || current_versions != prepared_versions
                || current_classification != preview.classification()
                || !evidence_matches_prepared
            {
                reject!(
                    "relationship.removal.execution_rejected",
                    AuditPolicyOutcome::Allowed,
                    AuditApprovalOutcome::Approved,
                    AuditExecutionOutcome::Failed,
                    preview_expired_or_changed(&context)
                );
            }

            let semantic_holder: Option<String> = tx
                .query_row(
                    "SELECT r.id FROM relationships r JOIN relationship_endpoints e0 ON e0.relationship_id=r.id AND e0.ordinal=0 AND e0.target_type=?1 AND e0.target_id=?2 JOIN relationship_endpoints e1 ON e1.relationship_id=r.id AND e1.ordinal=1 AND e1.target_type=?3 AND e1.target_id=?4 WHERE r.kind=?5 AND r.purpose IS ?6",
                    rusqlite::params![
                        endpoint_identities.first().map(|(t, _)| t.as_str()).unwrap_or_default(),
                        endpoint_identities.first().map(|(_, i)| i.as_str()).unwrap_or_default(),
                        endpoint_identities.get(1).map(|(t, _)| t.as_str()).unwrap_or_default(),
                        endpoint_identities.get(1).map(|(_, i)| i.as_str()).unwrap_or_default(),
                        current_kind.as_persisted(),
                        current_purpose.map(|value| value.as_persisted()),
                    ],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| storage_error(&context))?;
            if semantic_holder.as_deref() != Some(preview.relationship_id().as_str()) {
                reject!(
                    "relationship.removal.execution_rejected",
                    AuditPolicyOutcome::Allowed,
                    AuditApprovalOutcome::Approved,
                    AuditExecutionOutcome::Failed,
                    preview_expired_or_changed(&context)
                );
            }

            let rebuilt_preview = RemoveRelationshipPreview::from_persistence(
                command.prepared_id.clone(),
                preview.relationship_id().clone(),
                preview.relationship_version(),
                current_endpoints,
                current_kind,
                current_purpose,
                preview.effects().to_vec(),
                current_classification,
                RemovalPolicyDecision::Allowed,
                current_evidence.ok_or_else(|| storage_error(&context))?,
                preview.expires_at(),
                CancellationPolicy::NotCancellableAfterSubmit,
                preview.confirmation_challenge().to_owned(),
            );
            let recomputed = PreparedIntent::from_persistence_preview(rebuilt_preview.clone());
            if rebuilt_preview != *preview
                || recomputed.payload_digest() != prepared.payload_digest()
                || recomputed.payload_digest().as_str() != command.acknowledged_payload_digest.as_str()
            {
                reject!(
                    "relationship.removal.execution_rejected",
                    AuditPolicyOutcome::Allowed,
                    AuditApprovalOutcome::Approved,
                    AuditExecutionOutcome::Failed,
                    preview_expired_or_changed(&context)
                );
            }

            let occurred_millis = occurred_at.unix_millis();
            let relationship_id = preview.relationship_id().clone();
            tx.execute(
                "DELETE FROM relationship_endpoints WHERE relationship_id=?1",
                [relationship_id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "DELETE FROM relationships WHERE id=?1",
                [relationship_id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "DELETE FROM aggregate_registry WHERE id=?1 AND aggregate_type='relationship'",
                [relationship_id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;
            tombstone_relationship_history(tx, relationship_id.as_str(), command.prepared_id.as_str(), &context)?;
            tx.execute(
                "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2",
                rusqlite::params![occurred_millis, command.prepared_id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;

            let audit = build_execution_audit(
                audit_event_id,
                occurred_at,
                AuditTarget::Relationship(relationship_id.clone()),
                "relationship.removed",
                command.actor,
                AuditPolicyOutcome::Allowed,
                AuditApprovalOutcome::Approved,
                AuditExecutionOutcome::Succeeded,
                AuditEffectScope::Complete,
                vec!["relationship.authoritative-record-removed"],
                &context,
            )?;
            persist_execution_audit(tx, &audit, &context)?;
            let ordinal = next_relationship_operation_ordinal(tx, &context)?;
            tx.execute(
                "INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES(?1,'execute_remove',?2,?3,'removal',?4)",
                rusqlite::params![context.idempotency_id.as_str(), context.correlation_id.as_str(), ordinal, relationship_id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO relationship_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
                rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO relationship_h2b_command_results(idempotency_id,command_kind,result_kind,prepared_intent_id,actor,acknowledged_payload_digest,confirmation_digest,result_relationship_id) VALUES(?1,'execute_remove','removal',?2,'head_of_products',?3,?4,?5)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    command.prepared_id.as_str(),
                    command.acknowledged_payload_digest.as_str(),
                    confirmation_digest.as_str(),
                    relationship_id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;

            let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(&context))?;
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
            Ok(RemovalOutcome {
                relationship_id,
                audit_event_ids: vec![audit.id().clone()],
            })
        })
    }
}

fn tombstone_relationship_history(
    tx: &Transaction<'_>,
    relationship_id: &str,
    removal_prepared_intent_id: &str,
    context: &OperationContext,
) -> Result<(), DomainError> {
    let ordinary_ids: Vec<String> = {
        let mut statement = tx
            .prepare("SELECT idempotency_id FROM relationship_link_command_results WHERE relationship_id=?1")
            .map_err(|_| storage_error(context))?;
        let rows = statement
            .query_map([relationship_id], |row| row.get(0))
            .map_err(|_| storage_error(context))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| storage_error(context))?;
        rows
    };
    for idempotency_id in ordinary_ids {
        tx.execute(
            "INSERT OR IGNORE INTO relationship_replay_tombstones(idempotency_id,kind,removal_prepared_intent_id) VALUES(?1,'ordinary',?2)",
            rusqlite::params![idempotency_id, removal_prepared_intent_id],
        )
        .map_err(|_| storage_error(context))?;
    }
    let prepared_ids: Vec<String> = {
        let mut statement = tx
            .prepare("SELECT idempotency_id FROM relationship_h2b_command_results WHERE command_kind='prepare_remove' AND relationship_id=?1")
            .map_err(|_| storage_error(context))?;
        let rows = statement
            .query_map([relationship_id], |row| row.get(0))
            .map_err(|_| storage_error(context))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| storage_error(context))?;
        rows
    };
    for idempotency_id in prepared_ids {
        tx.execute(
            "INSERT OR IGNORE INTO relationship_replay_tombstones(idempotency_id,kind,removal_prepared_intent_id) VALUES(?1,'h2b',?2)",
            rusqlite::params![idempotency_id, removal_prepared_intent_id],
        )
        .map_err(|_| storage_error(context))?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn reject_execute(
    tx: &Transaction<'_>,
    context: &OperationContext,
    relationship_id: RelationshipId,
    actor: AuditActor,
    code: &str,
    policy_outcome: AuditPolicyOutcome,
    approval_outcome: AuditApprovalOutcome,
    execution_outcome: AuditExecutionOutcome,
    rejection: DomainError,
    command: &ApproveAndExecuteRemoveRelationship,
    confirmation_digest: &str,
    audit_event_id: AuditEventId,
    occurred_at: UtcTimestamp,
    expected_revision: u64,
) -> Result<RemovalOutcome, DomainError> {
    let audit = build_execution_audit(
        audit_event_id,
        occurred_at,
        AuditTarget::Relationship(relationship_id),
        code,
        actor,
        policy_outcome,
        approval_outcome,
        execution_outcome,
        AuditEffectScope::None,
        Vec::new(),
        context,
    )?;
    persist_execution_audit(tx, &audit, context)?;
    let ordinal = next_relationship_operation_ordinal(tx, context)?;
    tx.execute(
        "INSERT INTO relationship_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,error_code,error_message_key,error_retryable) VALUES(?1,'execute_remove',?2,?3,'rejection',NULL,?4,?5,?6)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            context.correlation_id.as_str(),
            ordinal,
            rejection.code().as_str(),
            rejection.message_key().as_str(),
            rejection.retryable(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO relationship_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES(?1,0,?2,?3)",
        rusqlite::params![context.idempotency_id.as_str(), audit.id().as_str(), context.correlation_id.as_str()],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO relationship_h2b_command_results(idempotency_id,command_kind,result_kind,prepared_intent_id,actor,acknowledged_payload_digest,confirmation_digest) VALUES(?1,'execute_remove','rejection',?2,'head_of_products',?3,?4)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            command.prepared_id.as_str(),
            command.acknowledged_payload_digest.as_str(),
            confirmation_digest,
        ],
    )
    .map_err(|_| storage_error(context))?;
    let expected_revision = i64::try_from(expected_revision).map_err(|_| storage_error(context))?;
    if tx
        .execute(
            "UPDATE ledger_metadata SET ledger_revision=?1 WHERE singleton=1 AND ledger_revision=?2",
            rusqlite::params![expected_revision + 1, expected_revision],
        )
        .map_err(|_| storage_error(context))?
        != 1
    {
        return Err(storage_error(context));
    }
    Err(rejection)
}

fn decode_execute_outcome(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<RemovalOutcome, DomainError> {
    let (result_kind, result_relationship_id): (String, Option<String>) = tx
        .query_row(
            "SELECT result_kind,result_relationship_id FROM relationship_h2b_command_results WHERE idempotency_id=?1 AND command_kind='execute_remove'",
            [context.idempotency_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|_| storage_error(context))?;
    if result_kind == "rejection" {
        let (error_code, error_message_key, error_retryable): (String, String, bool) = tx
            .query_row(
                "SELECT error_code,error_message_key,error_retryable FROM relationship_replay_operations WHERE idempotency_id=?1",
                [context.idempotency_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| storage_error(context))?;
        let code = match error_code.as_str() {
            "VALIDATION_INVALID_FIELD" => ErrorCode::ValidationInvalidField,
            "DOMAIN_CONFLICT" => ErrorCode::DomainConflict,
            "DOMAIN_NOT_FOUND" => ErrorCode::DomainNotFound,
            "SECURITY_POLICY_DENIED" => ErrorCode::SecurityPolicyDenied,
            "AI_POLICY_DENIED" => ErrorCode::AiPolicyDenied,
            "SECURITY_PREVIEW_EXPIRED_OR_CHANGED" => ErrorCode::SecurityPreviewExpiredOrChanged,
            "DOMAIN_IDEMPOTENCY_CONFLICT" => ErrorCode::DomainIdempotencyConflict,
            _ => ErrorCode::PlatformInternal,
        };
        let key = MessageKey::parse(error_message_key).map_err(|_| storage_error(context))?;
        return Err(DomainError::new(
            code,
            key,
            context.correlation_id.clone(),
            error_retryable,
        ));
    }
    let relationship_id = result_relationship_id.ok_or_else(|| storage_error(context))?;
    let audit_event_ids: Vec<AuditEventId> = {
        let mut statement = tx
            .prepare("SELECT audit_event_id FROM relationship_replay_audits WHERE idempotency_id=?1 ORDER BY ordinal")
            .map_err(|_| storage_error(context))?;
        let rows = statement
            .query_map([context.idempotency_id.as_str()], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|_| storage_error(context))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| storage_error(context))?;
        rows.into_iter()
            .map(|id| AuditEventId::parse(id).map_err(|_| storage_error(context)))
            .collect::<Result<_, _>>()?
    };
    Ok(RemovalOutcome {
        relationship_id: RelationshipId::parse(relationship_id)
            .map_err(|_| storage_error(context))?,
        audit_event_ids,
    })
}

/// `PayloadDigest::for_confirmation` is `pub(crate)` to `pmc-domain` and
/// unreachable here. Unlike the preview digest (which the domain compares
/// against a caller-supplied value and therefore must match exactly), this
/// confirmation digest is only ever compared against itself within this
/// table's own replay check -- nothing outside this writer ever computes or
/// verifies it -- so any deterministic hash is a faithful substitute.
fn confirmation_digest_for(confirmation: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"pmc-ledger.relationship.removal.confirmation.v1");
    hasher.update((confirmation.len() as u64).to_be_bytes());
    hasher.update(confirmation.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn authorization_denied(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::SecurityPolicyDenied,
        MessageKey::parse("relationship.removal.authorization_denied")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn confirmation_mismatch(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::SecurityPolicyDenied,
        MessageKey::parse("relationship.removal.confirmation_mismatch")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn preview_expired_or_changed(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::SecurityPreviewExpiredOrChanged,
        MessageKey::parse("relationship.removal.preview_expired_or_changed")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn persist_prepared_intent(
    tx: &Transaction<'_>,
    prepared: &PreparedIntent,
    occurred_millis: i64,
    context: &OperationContext,
) -> Result<(), DomainError> {
    let preview = prepared.preview();
    tx.execute(
        "INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,confirmation_challenge,expires_at,created_at) VALUES(?1,1,'relationship.remove',?2,?3,'allowed','not_cancellable_after_submit','head_of_products',?4,?5,?6)",
        rusqlite::params![
            prepared.id().as_str(),
            prepared.payload_digest().as_str(),
            preview.classification().as_persisted(),
            preview.confirmation_challenge(),
            preview.expires_at().unix_millis(),
            occurred_millis,
        ],
    )
    .map_err(|_| storage_error(context))?;
    tx.execute(
        "INSERT INTO prepared_removal_payloads(prepared_intent_id,relationship_id,relationship_version,relationship_kind,purpose) VALUES(?1,?2,?3,?4,?5)",
        rusqlite::params![
            prepared.id().as_str(),
            preview.relationship_id().as_str(),
            i64::try_from(preview.relationship_version().get()).map_err(|_| storage_error(context))?,
            preview.kind().as_persisted(),
            purpose_persisted_for_payload(preview.purpose()),
        ],
    )
    .map_err(|_| storage_error(context))?;
    for (ordinal, endpoint) in preview.endpoints().iter().enumerate() {
        let ordinal = i64::try_from(ordinal).map_err(|_| storage_error(context))?;
        let (endpoint_type, endpoint_id, endpoint_version, endpoint_classification) =
            endpoint_identity(endpoint);
        tx.execute(
            "INSERT INTO prepared_removal_endpoints(prepared_intent_id,ordinal,endpoint_type,endpoint_id,endpoint_version,classification,parent_project_id) VALUES(?1,?2,?3,?4,?5,?6,NULL)",
            rusqlite::params![
                prepared.id().as_str(),
                ordinal,
                endpoint_type,
                endpoint_id,
                i64::try_from(endpoint_version.get()).map_err(|_| storage_error(context))?,
                endpoint_classification.as_persisted(),
            ],
        )
        .map_err(|_| storage_error(context))?;
    }
    let evidence = preview.evidence();
    tx.execute(
        "INSERT INTO prepared_intent_recovery_evidence(prepared_intent_id,recovery_evidence_id,name,verified_at,relationship_id,compatible) VALUES(?1,?2,?3,?4,?5,?6)",
        rusqlite::params![
            prepared.id().as_str(),
            evidence.id().as_str(),
            evidence.name(),
            evidence.verified_at().unix_millis(),
            evidence.relationship_id().as_str(),
            evidence.compatible(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    for (ordinal, effect) in preview.effects().iter().enumerate() {
        let ordinal = i64::try_from(ordinal).map_err(|_| storage_error(context))?;
        tx.execute(
            "INSERT INTO prepared_intent_effects(prepared_intent_id,ordinal,effect_code) VALUES(?1,?2,?3)",
            rusqlite::params![prepared.id().as_str(), ordinal, removal_effect_persisted(*effect)],
        )
        .map_err(|_| storage_error(context))?;
    }
    Ok(())
}

fn purpose_persisted_for_payload(purpose: Option<StakeholderRelationshipPurpose>) -> &'static str {
    match purpose {
        None => "none",
        Some(StakeholderRelationshipPurpose::Responsibility) => "responsibility",
        Some(StakeholderRelationshipPurpose::Dependency) => "dependency",
    }
}

fn removal_effect_persisted(effect: RemovalEffect) -> &'static str {
    match effect {
        RemovalEffect::RemoveRelationshipRecord => "remove_relationship_record",
        RemovalEffect::RemoveSemanticRelationshipIndex => "remove_semantic_relationship_index",
        RemovalEffect::CreateIdempotencyTombstone => "create_idempotency_tombstone",
    }
}

fn endpoint_identity(
    endpoint: &pmc_domain::relationships::EndpointSnapshot,
) -> (&'static str, String, AggregateVersion, DataClassification) {
    use pmc_domain::relationships::EndpointSnapshot;
    match endpoint {
        EndpointSnapshot::Portfolio(v) => (
            "portfolio",
            v.id().as_str().to_owned(),
            v.version(),
            v.classification(),
        ),
        EndpointSnapshot::Product(v) => (
            "product",
            v.id().as_str().to_owned(),
            v.version(),
            v.classification(),
        ),
        EndpointSnapshot::Initiative(v) => (
            "initiative",
            v.id().as_str().to_owned(),
            v.version(),
            v.classification(),
        ),
        EndpointSnapshot::Roadmap(v) => (
            "roadmap",
            v.id().as_str().to_owned(),
            v.version(),
            v.classification(),
        ),
        EndpointSnapshot::Kpi(v) => (
            "kpi",
            v.id().as_str().to_owned(),
            v.version(),
            v.classification(),
        ),
        EndpointSnapshot::Project(v) => (
            "project",
            v.id().as_str().to_owned(),
            v.version(),
            v.classification(),
        ),
        EndpointSnapshot::Milestone(v) => (
            "milestone",
            v.id().as_str().to_owned(),
            v.version(),
            v.classification(),
        ),
        EndpointSnapshot::Stakeholder(v) => (
            "stakeholder",
            v.id().as_str().to_owned(),
            v.version(),
            v.classification(),
        ),
    }
}

fn decode_prepared_intent_outcome(
    tx: &Transaction<'_>,
    operation: &str,
    context: &OperationContext,
) -> Result<PreparedIntent, DomainError> {
    let prepared_intent_id: String = tx
        .query_row(
            "SELECT result_prepared_intent_id FROM relationship_h2b_command_results WHERE idempotency_id=?1 AND command_kind=?2",
            rusqlite::params![context.idempotency_id.as_str(), operation],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    decode_prepared_intent(tx, &prepared_intent_id, context)
}

fn decode_prepared_intent(
    tx: &Transaction<'_>,
    prepared_intent_id: &str,
    context: &OperationContext,
) -> Result<PreparedIntent, DomainError> {
    let (payload_digest, confirmation_challenge, expires_at): (String, String, i64) = tx
        .query_row(
            "SELECT payload_digest,confirmation_challenge,expires_at FROM prepared_intents WHERE id=?1",
            [prepared_intent_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| storage_error(context))?;
    let (relationship_id, relationship_version, relationship_kind, purpose): (String, i64, String, String) = tx
        .query_row(
            "SELECT relationship_id,relationship_version,relationship_kind,purpose FROM prepared_removal_payloads WHERE prepared_intent_id=?1",
            [prepared_intent_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| storage_error(context))?;
    let kind =
        RelationshipKind::from_persisted(&relationship_kind).map_err(|_| storage_error(context))?;
    let purpose = match purpose.as_str() {
        "none" => None,
        "responsibility" => Some(StakeholderRelationshipPurpose::Responsibility),
        "dependency" => Some(StakeholderRelationshipPurpose::Dependency),
        _ => return Err(storage_error(context)),
    };
    let endpoint_rows: Vec<(String, String, i64, String)> = {
        let mut statement = tx
            .prepare("SELECT endpoint_type,endpoint_id,endpoint_version,classification FROM prepared_removal_endpoints WHERE prepared_intent_id=?1 ORDER BY ordinal")
            .map_err(|_| storage_error(context))?;
        let rows = statement
            .query_map([prepared_intent_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|_| storage_error(context))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| storage_error(context))?;
        rows
    };
    let mut endpoints = Vec::with_capacity(endpoint_rows.len());
    for (endpoint_type, endpoint_id, endpoint_version, classification) in endpoint_rows {
        let aggregate_type = link_type_to_aggregate_type(&endpoint_type);
        let version = AggregateVersion::new(
            u64::try_from(endpoint_version).map_err(|_| storage_error(context))?,
        )
        .map_err(|_| storage_error(context))?;
        let classification = DataClassification::from_persisted(&classification)
            .map_err(|_| storage_error(context))?;
        endpoints.push(build_endpoint_snapshot(
            aggregate_type,
            &endpoint_id,
            version,
            classification,
            context,
        )?);
    }
    let (recovery_evidence_id, evidence_name, verified_at, evidence_relationship_id, compatible): (
        String,
        String,
        i64,
        String,
        bool,
    ) = tx
        .query_row(
            "SELECT recovery_evidence_id,name,verified_at,relationship_id,compatible FROM prepared_intent_recovery_evidence WHERE prepared_intent_id=?1",
            [prepared_intent_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .map_err(|_| storage_error(context))?;
    let evidence = RecoveryEvidence::new(
        pmc_domain::identity::RecoveryEvidenceId::parse(recovery_evidence_id)
            .map_err(|_| storage_error(context))?,
        evidence_name,
        UtcTimestamp::from_unix_millis(verified_at),
        RelationshipId::parse(evidence_relationship_id).map_err(|_| storage_error(context))?,
        compatible,
    )
    .map_err(|_| storage_error(context))?;
    let stored_classification: String = tx
        .query_row(
            "SELECT classification FROM prepared_intents WHERE id=?1",
            [prepared_intent_id],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    let preview = RemoveRelationshipPreview::from_persistence(
        PreparedIntentId::parse(prepared_intent_id).map_err(|_| storage_error(context))?,
        RelationshipId::parse(relationship_id).map_err(|_| storage_error(context))?,
        AggregateVersion::new(
            u64::try_from(relationship_version).map_err(|_| storage_error(context))?,
        )
        .map_err(|_| storage_error(context))?,
        endpoints,
        kind,
        purpose,
        vec![
            RemovalEffect::RemoveRelationshipRecord,
            RemovalEffect::RemoveSemanticRelationshipIndex,
            RemovalEffect::CreateIdempotencyTombstone,
        ],
        DataClassification::from_persisted(&stored_classification)
            .map_err(|_| storage_error(context))?,
        RemovalPolicyDecision::Allowed,
        evidence,
        UtcTimestamp::from_unix_millis(expires_at),
        CancellationPolicy::NotCancellableAfterSubmit,
        confirmation_challenge,
    );
    let digest = pmc_domain::execution::PayloadDigest::parse(payload_digest)
        .map_err(|_| storage_error(context))?;
    Ok(PreparedIntent::from_persistence(preview, digest))
}

/// H2b's own audit family, matching `relationships.rs::append_removal_audit`/
/// `append_cancellation_audit`/`append_removal_rejection_audit` exactly:
/// `AuditModule::Execution` (not `Portfolio`, unlike every ordinary H1
/// Relationship audit built in `relationship_repository.rs`), a caller-given
/// disposition per call site, and no fixed effect code (an empty effect list
/// for cancellation/rejection; the removal effect code is passed by the
/// execute caller).
#[allow(clippy::too_many_arguments)]
fn build_execution_audit(
    id: AuditEventId,
    at: UtcTimestamp,
    target: AuditTarget,
    code: &str,
    actor: AuditActor,
    policy_outcome: AuditPolicyOutcome,
    approval_outcome: AuditApprovalOutcome,
    execution_outcome: AuditExecutionOutcome,
    effect_scope: AuditEffectScope,
    effect_codes: Vec<&str>,
    context: &OperationContext,
) -> Result<pmc_domain::audit::AuditEvent, DomainError> {
    let effects = effect_codes
        .into_iter()
        .map(|code| {
            pmc_domain::audit::AuditEffectCode::parse(code).map_err(|_| storage_error(context))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(pmc_domain::audit::AuditEvent::new(
        id,
        at,
        actor,
        pmc_domain::audit::AuditAction::new(
            AuditModule::Execution,
            AuditEventCode::parse(code).map_err(|_| storage_error(context))?,
            target,
        ),
        context.correlation_id.clone(),
        AuditDisposition::new(
            policy_outcome,
            approval_outcome,
            execution_outcome,
            effect_scope,
            effects,
        )
        .map_err(|_| storage_error(context))?,
    ))
}

fn persist_execution_audit(
    tx: &Transaction<'_>,
    audit: &pmc_domain::audit::AuditEvent,
    context: &OperationContext,
) -> Result<(), DomainError> {
    let (target_type, target_id) = match audit.target() {
        AuditTarget::Relationship(id) => ("relationship", id.as_str().to_owned()),
        _ => return Err(storage_error(context)),
    };
    let effect_scope = match audit.effect_scope() {
        AuditEffectScope::None => "none",
        AuditEffectScope::Partial => "partial",
        AuditEffectScope::Complete => "complete",
    };
    let policy_outcome = match audit.policy_outcome() {
        AuditPolicyOutcome::NotRequired => "not_required",
        AuditPolicyOutcome::Allowed => "allowed",
        AuditPolicyOutcome::Denied => "denied",
    };
    let approval_outcome = match audit.approval_outcome() {
        AuditApprovalOutcome::NotRequired => "not_required",
        AuditApprovalOutcome::Approved => "approved",
        AuditApprovalOutcome::Rejected => "rejected",
    };
    let execution_outcome = match audit.execution_outcome() {
        AuditExecutionOutcome::NotAttempted => "not_attempted",
        AuditExecutionOutcome::Succeeded => "succeeded",
        AuditExecutionOutcome::Failed => "failed",
        AuditExecutionOutcome::Cancelled => "cancelled",
    };
    let actor = match audit.actor() {
        AuditActor::HeadOfProducts => "head_of_products",
        AuditActor::PolicyAuthorizedSystem => "policy_authorized_system",
    };
    tx.execute(
        "INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES(?1,?2,?3,'execution',?4,?5,?6,?7,?8,?9,?10,?11)",
        rusqlite::params![
            audit.id().as_str(),
            audit.occurred_at().unix_millis(),
            actor,
            audit.code().as_str(),
            target_type,
            target_id,
            context.correlation_id.as_str(),
            policy_outcome,
            approval_outcome,
            execution_outcome,
            effect_scope,
        ],
    )
    .map_err(|_| storage_error(context))?;
    for (ordinal, effect) in audit.actual_effects().iter().enumerate() {
        let ordinal = i64::try_from(ordinal).map_err(|_| storage_error(context))?;
        tx.execute(
            "INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![
                audit.id().as_str(),
                ordinal,
                effect.as_str(),
                effect_scope,
                target_type,
                target_id,
            ],
        )
        .map_err(|_| storage_error(context))?;
    }
    Ok(())
}

fn too_late_to_cancel(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("relationship.removal.too_late_to_cancel")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}
