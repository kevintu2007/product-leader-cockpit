//! SQLite persistence for the managed-projection rebuild H2a mechanism
//! over the V41 schema.
//!
//! One deliberate departure from every other H2a repository here: this
//! module does **not** advance `ledger_metadata.ledger_revision`. Every
//! other mechanism does, because it mutates a Ledger aggregate. A
//! projection rebuild mutates none -- it only publishes files derived from
//! whatever revision already exists. Advancing it would be actively
//! broken, not merely redundant: the preview binds the revision it was
//! computed from, `execute_rebuild_managed_projections` rejects any
//! revision change as drift, and the generated files embed that revision.
//! A prepare that bumped the revision would invalidate its own preview the
//! moment it committed, so execute could never succeed.

use pmc_domain::{
    classification::DataClassification,
    error::{DomainError, ErrorCode, MessageKey},
    identity::{ApprovalReceiptId, PreparedIntentId},
    managed_projection_rebuild::{
        execute_rebuild_managed_projections as execute_rebuild,
        prepare_rebuild_managed_projections as prepare_rebuild,
        ApproveAndExecuteRebuildManagedProjections, ManagedProjectionEffectScope,
        ManagedProjectionH1AutoAuthorization, ManagedProjectionPublishReport,
        ManagedProjectionRebuildChangeEntry, ManagedProjectionRebuildChangeKind,
        ManagedProjectionRebuildEffects, ManagedProjectionRebuildError,
        ManagedProjectionRebuildPayloadDigest, ManagedProjectionRebuildPreparedIntent,
        ManagedProjectionRebuildPreview, ManagedProjectionRebuildRecordKind, OperationContext,
        PrepareRebuildManagedProjections,
    },
    time::UtcTimestamp,
    work_management::WorkManagementRationale,
};
use rusqlite::{OptionalExtension, Transaction};

use super::{LedgerTransactionError, SqliteProductLedger};

impl SqliteProductLedger {
    /// H2a step 1: durably prepare a managed-projection rebuild.
    ///
    /// Delegates preview construction, canonical ordering validation and
    /// digest computation to the pure domain function, then persists the
    /// prepared intent, its payload, its ordered change set, and the
    /// idempotency echo used to detect a conflicting reuse of the same
    /// identifier. Replaying the same `idempotency_id` with identical
    /// inputs returns the original prepared intent rather than preparing
    /// again.
    pub fn prepare_rebuild_managed_projections(
        &mut self,
        command: PrepareRebuildManagedProjections,
        prepared_intent_id: PreparedIntentId,
        now: UtcTimestamp,
    ) -> Result<ManagedProjectionRebuildPreparedIntent, LedgerTransactionError<DomainError>> {
        let context = command.context.clone();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing_prepared_id) = existing_prepare(tx, &context)? {
                return if prepare_inputs_match(tx, &existing_prepared_id, &command, &context)? {
                    decode_prepared_intent(tx, &existing_prepared_id, &context)
                } else {
                    Err(idempotency_conflict(&context))
                };
            }
            let prepared = prepare_rebuild(&command, prepared_intent_id, now)
                .map_err(|error| rebuild_domain_error(&error, &context))?;
            persist_prepared_intent(tx, &prepared, &context)?;
            persist_prepare_command_echo(tx, &command, &context)?;
            let ordinal = next_prepare_ordinal(tx, &context)?;
            tx.execute(
                "INSERT INTO rebuild_managed_projections_prepare_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES (?1,'prepare_rebuild_managed_projections',?2,?3,'prepared',?4)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared.id().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            Ok(prepared)
        })
    }

    /// H2a step 2: approve a prepared rebuild and durably open the
    /// publication.
    ///
    /// This is the SHORT Ledger transaction from the approved ordering:
    /// revalidate against current state, consume the Prepared Intent, mint
    /// an already-consumed Approval Receipt, claim the execute idempotency,
    /// and record the complete planned item set as `publishing` with every
    /// path `untouched`. It writes no files. The caller publishes outside
    /// this transaction and then calls
    /// [`Self::complete_rebuild_managed_projections`] -- so a crash between
    /// the two leaves a durable `publishing` operation whose item rows say
    /// exactly what was intended, rather than an invisible half-state.
    ///
    /// `current_changes` and `current_published_baseline_digest` come from
    /// the caller because computing them needs the projection generator,
    /// which this crate cannot reach. The Ledger revision and schema
    /// version are read here rather than accepted, so a caller cannot
    /// assert a revision the Ledger does not actually hold.
    pub fn approve_and_execute_rebuild_managed_projections(
        &mut self,
        command: ApproveAndExecuteRebuildManagedProjections,
        approval_receipt_id: ApprovalReceiptId,
        current_published_baseline_digest: String,
        current_changes: Vec<ManagedProjectionRebuildChangeEntry>,
        now: UtcTimestamp,
    ) -> Result<ManagedProjectionRebuildEffects, LedgerTransactionError<DomainError>> {
        let context = command.context.clone();
        let schema_version = self.schema_version();
        self.with_immediate_transaction(|transaction| {
            let tx = &mut transaction.transaction;
            if let Some(existing) = existing_execute(tx, &context)? {
                // Every element the approval asserted has to match, not just
                // the prepared identifier. A replay that agrees on the intent
                // but disagrees on the digest, the actor or the correlation is
                // a different request wearing the same key.
                let matches = existing.prepared_id == command.approval.prepared_id().as_str()
                    && existing.acknowledged_digest
                        == command.approval.acknowledged_payload_digest().as_str()
                    && existing.actor == command.approval.actor().as_persisted()
                    && existing.correlation_id == context.correlation_id.as_str();
                return if matches {
                    Ok(ManagedProjectionRebuildEffects {
                        changes: load_changes(tx, &existing.prepared_id, &context)?,
                    })
                } else {
                    Err(idempotency_conflict(&context))
                };
            }
            let prepared_id = command.approval.prepared_id().as_str().to_owned();
            if is_consumed(tx, &prepared_id, &context)? {
                return Err(prepared_intent_already_consumed(&context));
            }
            let prepared = decode_prepared_intent(tx, &prepared_id, &context)?;
            let current_revision = read_revision(tx, &context)?;
            let effects = execute_rebuild(
                &command.approval,
                &prepared,
                schema_version,
                current_revision,
                &current_published_baseline_digest,
                &current_changes,
                now,
            )
            .map_err(|error| rebuild_domain_error(&error, &context))?;

            if tx
                .execute(
                    "UPDATE prepared_intents SET consumed_at=?1 WHERE id=?2 AND consumed_at IS NULL",
                    rusqlite::params![now.unix_millis(), prepared_id.as_str()],
                )
                .map_err(|_| storage_error(&context))?
                != 1
            {
                return Err(prepared_intent_already_consumed(&context));
            }
            // Minted already consumed: the durable publication begins in
            // this same transaction, so leaving consumed_at NULL would
            // describe a receipt that is still spendable.
            if publication_in_flight(tx, &context)? {
                return Err(rebuild_domain_error(
                    &ManagedProjectionRebuildError::PublicationInFlight,
                    &context,
                ));
            }
            tx.execute(
                "INSERT INTO approval_receipts (id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at) VALUES (?1,?2,'head_of_products',?3,?4,?5,?6,?5)",
                rusqlite::params![
                    approval_receipt_id.as_str(),
                    prepared_id.as_str(),
                    command.approval.acknowledged_payload_digest().as_str(),
                    context.idempotency_id.as_str(),
                    now.unix_millis(),
                    prepared.preview().expires_at.unix_millis(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            tx.execute(
                "INSERT INTO rebuild_managed_projections_execute_commands (idempotency_id,authorization,prepared_id,actor,acknowledged_digest) VALUES (?1,'h2a',?2,'head_of_products',?3)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    prepared_id.as_str(),
                    command.approval.acknowledged_payload_digest().as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            let ordinal = next_execute_ordinal(tx, &context)?;
            tx.execute(
                "INSERT INTO rebuild_managed_projections_execute_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,authorization,status,effect_scope,prepared_intent_id,approval_receipt_id,started_at,completed_at) VALUES (?1,'execute_rebuild_managed_projections',?2,?3,'h2a','publishing',NULL,?4,?5,?6,NULL)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    prepared_id.as_str(),
                    approval_receipt_id.as_str(),
                    now.unix_millis(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_operation_items(tx, &context, &effects.changes)?;
            mark_head_out_of_sync(tx, now, &context)?;
            Ok(effects)
        })
    }

    /// Opens an H1-Auto publication -- DG0's
    /// `RebuildManagedProjectionsIncremental` route, which is pre-authorized
    /// by policy and therefore has no Prepared Intent and no Approval
    /// Receipt.
    ///
    /// The `authorization` argument is a witness, not a flag: it can only be
    /// constructed by re-checking ADR 0007's ceilings, so this method cannot
    /// be told that oversized work is automatable. Its change count must
    /// also match the set actually being recorded, so a small authorization
    /// cannot be reused to admit a large publication.
    ///
    /// Otherwise identical in shape to the H2a route: the operation opens as
    /// `publishing` with every path `untouched`, files are written outside
    /// this transaction, and
    /// [`Self::complete_rebuild_managed_projections`] makes it terminal.
    pub fn begin_h1_auto_rebuild_managed_projections(
        &mut self,
        context: OperationContext,
        authorization: ManagedProjectionH1AutoAuthorization,
        changes: Vec<ManagedProjectionRebuildChangeEntry>,
        now: UtcTimestamp,
    ) -> Result<ManagedProjectionRebuildEffects, LedgerTransactionError<DomainError>> {
        self.with_immediate_transaction(move |transaction| {
            let tx = &mut transaction.transaction;
            if authorization.change_count() != changes.len() {
                return Err(rebuild_domain_error(
                    &ManagedProjectionRebuildError::H1AutoNotPermitted,
                    &context,
                ));
            }
            if changes.is_empty() {
                return Err(rebuild_domain_error(
                    &ManagedProjectionRebuildError::EmptyChangeSet,
                    &context,
                ));
            }
            if changes
                .windows(2)
                .any(|pair| pair[0].relative_path >= pair[1].relative_path)
            {
                return Err(rebuild_domain_error(
                    &ManagedProjectionRebuildError::ChangesNotCanonical,
                    &context,
                ));
            }
            // ADR 0007 forbids batching around approval, and a per-operation
            // ceiling alone cannot enforce that: two authorizations of 500
            // accomplish 1000 files' work with no approval anywhere. The
            // invariant that actually closes it is that automation may only
            // ever run from a synchronized baseline.
            //
            // A split cannot survive it. Publishing half a diff leaves the
            // other half stale on disk, so the whole-set verification in
            // `attach_verified_manifest` fails, no manifest is minted, and
            // the head stays out of sync -- which refuses the second half
            // here and forces it to H2a. The baseline can never advance
            // mid-split, so there is no sequence of sub-ceiling operations
            // that adds up to unapproved work.
            //
            // It also stops the caller asserting away the two booleans the
            // witness has to take on faith: an absent head is an initial
            // rebuild, and an out-of-sync head is an integrity conflict.
            // Both already require H2a, and the Ledger now checks rather
            // than believes.
            if !projection_head_is_verified(tx, &context)? {
                return Err(rebuild_domain_error(
                    &ManagedProjectionRebuildError::H1AutoNotPermitted,
                    &context,
                ));
            }
            if existing_execute_operation(tx, &context)? {
                return Err(idempotency_conflict(&context));
            }
            if publication_in_flight(tx, &context)? {
                return Err(rebuild_domain_error(
                    &ManagedProjectionRebuildError::PublicationInFlight,
                    &context,
                ));
            }
            tx.execute(
                "INSERT INTO rebuild_managed_projections_execute_commands (idempotency_id,authorization,prepared_id,actor,acknowledged_digest) VALUES (?1,'h1_auto',NULL,NULL,NULL)",
                rusqlite::params![context.idempotency_id.as_str()],
            )
            .map_err(|_| storage_error(&context))?;
            let ordinal = next_execute_ordinal(tx, &context)?;
            tx.execute(
                "INSERT INTO rebuild_managed_projections_execute_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,authorization,status,effect_scope,prepared_intent_id,approval_receipt_id,started_at,completed_at) VALUES (?1,'execute_rebuild_managed_projections',?2,?3,'h1_auto','publishing',NULL,NULL,NULL,?4,NULL)",
                rusqlite::params![
                    context.idempotency_id.as_str(),
                    context.correlation_id.as_str(),
                    ordinal,
                    now.unix_millis(),
                ],
            )
            .map_err(|_| storage_error(&context))?;
            persist_operation_items(tx, &context, &changes)?;
            mark_head_out_of_sync(tx, now, &context)?;
            Ok(ManagedProjectionRebuildEffects { changes })
        })
    }

    /// Requests cancellation of a publication in flight.
    ///
    /// ADR 0006 shows `Cancelling` until a terminal outcome, so this only
    /// moves `publishing` -> `cancelling`; it never invents a terminal state
    /// and never touches the item rows. The publisher observes the request
    /// at its next safe boundary and stops there, and
    /// [`Self::complete_rebuild_managed_projections`] then records what
    /// actually landed. Requesting cancellation twice is idempotent.
    pub fn request_rebuild_managed_projections_cancellation(
        &mut self,
        context: OperationContext,
    ) -> Result<(), LedgerTransactionError<DomainError>> {
        self.with_immediate_transaction(move |transaction| {
            let tx = &mut transaction.transaction;
            let status: String = tx
                .query_row(
                    "SELECT status FROM rebuild_managed_projections_execute_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| unknown_operation(&context))?;
            match status.as_str() {
                "cancelling" => Ok(()),
                "publishing" => {
                    tx.execute(
                        "UPDATE rebuild_managed_projections_execute_replay_operations SET status='cancelling' WHERE idempotency_id=?1",
                        [context.idempotency_id.as_str()],
                    )
                    .map_err(|_| storage_error(&context))?;
                    Ok(())
                }
                _ => Err(operation_already_terminal(&context)),
            }
        })
    }

    /// Whether cancellation has been requested for a publication in flight.
    /// After an Operational Restore (S7 plan §7): the Ledger came back from
    /// an archive but the projection files on disk did not, so the head
    /// cannot claim Synchronized. Only a completed, integrity-checked rebuild
    /// (ADR 0007) makes it so again.
    pub fn mark_projections_out_of_sync_after_restore(
        &mut self,
        now: UtcTimestamp,
    ) -> Result<(), LedgerTransactionError<()>> {
        self.with_immediate_transaction(|transaction| {
            transaction
                .transaction
                .execute(
                    "INSERT INTO projection_manifest_head (singleton,manifest_id,integrity_state,updated_at) VALUES (1,NULL,'out_of_sync',?1) ON CONFLICT(singleton) DO UPDATE SET integrity_state='out_of_sync',updated_at=excluded.updated_at",
                    rusqlite::params![now.unix_millis()],
                )
                .map(|_| ())
                .map_err(|_| ())
        })
    }

    /// The publisher polls this at each safe boundary.
    #[must_use]
    pub fn rebuild_managed_projections_cancellation_requested(&self, idempotency_id: &str) -> bool {
        self.connection
            .query_row(
                "SELECT status FROM rebuild_managed_projections_execute_replay_operations WHERE idempotency_id=?1",
                [idempotency_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
            .is_some_and(|status| status == "cancelling")
    }

    /// Makes a publication terminal.
    ///
    /// The final Ledger transaction from the approved ordering: record what
    /// actually happened to each path, derive the effect scope from those
    /// item states rather than trusting a caller-supplied summary, and --
    /// only when the complete set verified on disk -- insert the new
    /// immutable manifest generation and atomically switch the head pointer
    /// to it as `verified`. A partial, failed or cancelled publication
    /// leaves the previous verified generation in place as the diff
    /// baseline and marks the head `out_of_sync`, which is what keeps
    /// "Synchronized" an integrity result rather than a publish-attempt
    /// result (ADR 0007).
    pub fn complete_rebuild_managed_projections(
        &mut self,
        context: OperationContext,
        report: ManagedProjectionPublishReport,
        now: UtcTimestamp,
    ) -> Result<ManagedProjectionEffectScope, LedgerTransactionError<DomainError>> {
        self.with_immediate_transaction(move |transaction| {
            let tx = &mut transaction.transaction;
            let status: String = tx
                .query_row(
                    "SELECT status FROM rebuild_managed_projections_execute_replay_operations WHERE idempotency_id=?1",
                    [context.idempotency_id.as_str()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| storage_error(&context))?
                .ok_or_else(|| unknown_operation(&context))?;
            if status != "publishing" && status != "cancelling" {
                return Err(operation_already_terminal(&context));
            }
            // The report must account for every planned path exactly once.
            // Without this the effect scope is derived from whatever subset
            // the caller chose to mention: reporting only the one path that
            // committed, out of two, yields `complete` while the other row
            // silently stays `untouched`. Refusing an under- or
            // over-covering report is what makes the scope a fact about the
            // operation rather than about the report.
            let planned = planned_item_count(tx, &context)?;
            let mut reported: Vec<&str> = report
                .items
                .iter()
                .map(|item| item.relative_path.as_str())
                .collect();
            reported.sort_unstable();
            let distinct = reported.windows(2).all(|pair| pair[0] != pair[1]);
            if !distinct || reported.len() != planned {
                return Err(incomplete_report(&context));
            }
            for item in &report.items {
                if tx
                    .execute(
                        "UPDATE rebuild_managed_projections_operation_items SET state=?1 WHERE idempotency_id=?2 AND relative_path=?3",
                        rusqlite::params![
                            item.state.as_persisted(),
                            context.idempotency_id.as_str(),
                            item.relative_path.as_str(),
                        ],
                    )
                    .map_err(|_| storage_error(&context))?
                    != 1
                {
                    // A path nobody planned must never acquire a result row:
                    // it would misreport what the approved operation touched.
                    return Err(unplanned_item(&context));
                }
            }
            let effect_scope = ManagedProjectionEffectScope::derive(&report.items);
            tx.execute(
                "UPDATE rebuild_managed_projections_execute_replay_operations SET status=?1,effect_scope=?2,completed_at=?3 WHERE idempotency_id=?4",
                rusqlite::params![
                    report.status.as_persisted(),
                    effect_scope.as_persisted(),
                    now.unix_millis(),
                    context.idempotency_id.as_str(),
                ],
            )
            .map_err(|_| storage_error(&context))?;

            match &report.manifest {
                Some(manifest) => {
                    tx.execute(
                        "INSERT INTO projection_manifest_generations (manifest_id,projection_schema,ledger_schema_version,ledger_revision,manifest_digest,verified_at,publication_idempotency_id) VALUES (?1,'pmc.projection/v1',?2,?3,?4,?5,?6)",
                        rusqlite::params![
                            manifest.manifest_id.as_str(),
                            i64::from(manifest.ledger_schema_version),
                            i64::try_from(manifest.ledger_revision)
                                .map_err(|_| storage_error(&context))?,
                            manifest.manifest_digest.as_str(),
                            now.unix_millis(),
                            context.idempotency_id.as_str(),
                        ],
                    )
                    .map_err(|_| storage_error(&context))?;
                    for entry in &manifest.entries {
                        tx.execute(
                            "INSERT INTO projection_manifest_entries (manifest_id,relative_path,record_kind,record_id,source_revision,classification,managed_payload_sha256,final_file_sha256) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                            rusqlite::params![
                                manifest.manifest_id.as_str(),
                                entry.relative_path.as_str(),
                                entry.record_kind.as_persisted(),
                                entry.record_id.as_str(),
                                i64::try_from(entry.source_revision)
                                    .map_err(|_| storage_error(&context))?,
                                entry.classification.as_persisted(),
                                entry.managed_payload_sha256.as_str(),
                                entry.final_file_sha256.as_str(),
                            ],
                        )
                        .map_err(|_| storage_error(&context))?;
                    }
                    tx.execute(
                        "INSERT INTO projection_manifest_head (singleton,manifest_id,integrity_state,updated_at) VALUES (1,?1,'verified',?2) ON CONFLICT(singleton) DO UPDATE SET manifest_id=excluded.manifest_id,integrity_state='verified',updated_at=excluded.updated_at",
                        rusqlite::params![manifest.manifest_id.as_str(), now.unix_millis()],
                    )
                    .map_err(|_| storage_error(&context))?;
                }
                None => {
                    // Keep whichever generation was last verified as the
                    // baseline; only the integrity state changes. Opening
                    // already marked it, so this is idempotent here.
                    mark_head_out_of_sync(tx, now, &context)?;
                }
            }
            Ok(effect_scope)
        })
    }
}

/// Everything the original execute bound, so a replay can be compared against
/// all of it rather than against one field.
struct ExecuteBinding {
    prepared_id: String,
    actor: String,
    acknowledged_digest: String,
    correlation_id: String,
}

/// Reads the full binding of a previous execute under this idempotency id.
///
/// An idempotency key has to bind everything the caller asserted, or a replay
/// silently accepts a different request as the original. Comparing only the
/// prepared identifier let a caller reuse a key with a different acknowledged
/// digest, actor or correlation and receive success. This is the same shape
/// as the replay gap already closed in the H2a Lower Data Classification
/// execute paths, which did not check the approval's idempotency id.
fn existing_execute(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<Option<ExecuteBinding>, DomainError> {
    tx.query_row(
        "SELECT o.prepared_intent_id,c.actor,c.acknowledged_digest,o.correlation_id \
         FROM rebuild_managed_projections_execute_replay_operations o \
         JOIN rebuild_managed_projections_execute_commands c USING (idempotency_id) \
         WHERE o.idempotency_id=?1",
        [context.idempotency_id.as_str()],
        |row| {
            Ok(ExecuteBinding {
                prepared_id: row.get(0)?,
                actor: row.get(1)?,
                acknowledged_digest: row.get(2)?,
                correlation_id: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(|_| storage_error(context))
}

fn is_consumed(
    tx: &Transaction<'_>,
    prepared_intent_id: &str,
    context: &OperationContext,
) -> Result<bool, DomainError> {
    tx.query_row(
        "SELECT consumed_at IS NOT NULL FROM prepared_intents WHERE id=?1",
        [prepared_intent_id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map_err(|_| storage_error(context))?
    .map(|flag| flag != 0)
    .ok_or_else(|| unknown_prepared_intent(context))
}

fn read_revision(tx: &Transaction<'_>, context: &OperationContext) -> Result<u64, DomainError> {
    let value: i64 = tx
        .query_row(
            "SELECT ledger_revision FROM ledger_metadata WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    u64::try_from(value).map_err(|_| storage_error(context))
}

fn next_execute_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM rebuild_managed_projections_execute_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn unknown_prepared_intent(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainNotFound,
        MessageKey::parse("projection.rebuild_prepared_intent_not_found")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn prepared_intent_already_consumed(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("projection.rebuild_prepared_intent_consumed")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn unknown_operation(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainNotFound,
        MessageKey::parse("projection.rebuild_operation_not_found")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn operation_already_terminal(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("projection.rebuild_operation_terminal")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn unplanned_item(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("projection.rebuild_unplanned_item").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn existing_prepare(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<Option<String>, DomainError> {
    tx.query_row(
        "SELECT result_reference FROM rebuild_managed_projections_prepare_replay_operations WHERE idempotency_id=?1",
        [context.idempotency_id.as_str()],
        |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|_| storage_error(context))
}

/// Compares every semantic input the caller supplied against what the
/// original prepare stored. The change set is compared in full, not just
/// the scalars: two rebuilds can share a Ledger revision and baseline yet
/// touch entirely different files, and returning the wrong prepared intent
/// would approve a publish nobody previewed.
fn prepare_inputs_match(
    tx: &Transaction<'_>,
    prepared_intent_id: &str,
    command: &PrepareRebuildManagedProjections,
    context: &OperationContext,
) -> Result<bool, DomainError> {
    let stored: (i64, i64, String, i64, String) = tx
        .query_row(
            "SELECT ledger_schema_version,ledger_revision,published_baseline_digest,estimated_duration_millis,rationale FROM prepared_rebuild_managed_projections_payloads WHERE prepared_intent_id=?1",
            [prepared_intent_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let scalars_match = u32::try_from(stored.0) == Ok(command.ledger_schema_version)
        && u64::try_from(stored.1) == Ok(command.ledger_revision)
        && stored.2 == command.published_baseline_digest
        && u64::try_from(stored.3) == Ok(command.estimated_duration_millis)
        && stored.4 == command.rationale.as_str();
    if !scalars_match {
        return Ok(false);
    }
    Ok(load_changes(tx, prepared_intent_id, context)? == command.changes)
}

fn load_changes(
    tx: &Transaction<'_>,
    prepared_intent_id: &str,
    context: &OperationContext,
) -> Result<Vec<ManagedProjectionRebuildChangeEntry>, DomainError> {
    tx.prepare(
        "SELECT relative_path,record_kind,record_id,change_kind,classification,expected_final_file_sha256 FROM prepared_rebuild_managed_projections_changes WHERE prepared_intent_id=?1 ORDER BY ordinal",
    )
    .map_err(|_| storage_error(context))?
    .query_map([prepared_intent_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
        ))
    })
    .map_err(|_| storage_error(context))?
    .map(|row| {
        let row = row.map_err(|_| storage_error(context))?;
        Ok(ManagedProjectionRebuildChangeEntry {
            relative_path: row.0,
            record_kind: ManagedProjectionRebuildRecordKind::from_persisted(&row.1)
                .map_err(|_| storage_error(context))?,
            record_id: row.2,
            change: ManagedProjectionRebuildChangeKind::from_persisted(&row.3)
                .map_err(|_| storage_error(context))?,
            classification: DataClassification::from_persisted(&row.4)
                .map_err(|_| storage_error(context))?,
            expected_final_file_sha256: row.5,
        })
    })
    .collect()
}

fn decode_prepared_intent(
    tx: &Transaction<'_>,
    prepared_intent_id: &str,
    context: &OperationContext,
) -> Result<ManagedProjectionRebuildPreparedIntent, DomainError> {
    let header: (String, String, i64, i64) = tx
        .query_row(
            "SELECT payload_digest,classification,expires_at,created_at FROM prepared_intents WHERE id=?1 AND intent_kind='rebuild_managed_projections'",
            [prepared_intent_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|_| storage_error(context))?;
    let payload: (i64, i64, String, i64, String) = tx
        .query_row(
            "SELECT ledger_schema_version,ledger_revision,published_baseline_digest,estimated_duration_millis,rationale FROM prepared_rebuild_managed_projections_payloads WHERE prepared_intent_id=?1",
            [prepared_intent_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .map_err(|_| storage_error(context))?;
    let preview = ManagedProjectionRebuildPreview {
        prepared_intent_id: PreparedIntentId::parse(prepared_intent_id)
            .map_err(|_| storage_error(context))?,
        ledger_schema_version: u32::try_from(payload.0).map_err(|_| storage_error(context))?,
        ledger_revision: u64::try_from(payload.1).map_err(|_| storage_error(context))?,
        published_baseline_digest: payload.2,
        changes: load_changes(tx, prepared_intent_id, context)?,
        classification: DataClassification::from_persisted(&header.1)
            .map_err(|_| storage_error(context))?,
        estimated_duration_millis: u64::try_from(payload.3).map_err(|_| storage_error(context))?,
        rationale: WorkManagementRationale::parse(payload.4).map_err(|_| storage_error(context))?,
        expires_at: UtcTimestamp::from_unix_millis(header.2),
    };
    let digest = ManagedProjectionRebuildPayloadDigest::from_persisted(header.0)
        .map_err(|_| storage_error(context))?;
    Ok(ManagedProjectionRebuildPreparedIntent::from_persisted(
        preview,
        digest,
        UtcTimestamp::from_unix_millis(header.3),
    ))
}

fn persist_prepared_intent(
    tx: &Transaction<'_>,
    prepared: &ManagedProjectionRebuildPreparedIntent,
    context: &OperationContext,
) -> Result<(), DomainError> {
    let preview = prepared.preview();
    tx.execute(
        "INSERT INTO prepared_intents (id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,expires_at,created_at) VALUES (?1,1,'rebuild_managed_projections',?2,?3,'allowed','cancellable_at_safe_boundaries','head_of_products',?4,?5)",
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
        "INSERT INTO prepared_rebuild_managed_projections_payloads (prepared_intent_id,ledger_schema_version,ledger_revision,published_baseline_digest,estimated_duration_millis,classification,rationale) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![
            prepared.id().as_str(),
            i64::from(preview.ledger_schema_version),
            i64::try_from(preview.ledger_revision).map_err(|_| storage_error(context))?,
            preview.published_baseline_digest.as_str(),
            i64::try_from(preview.estimated_duration_millis)
                .map_err(|_| storage_error(context))?,
            preview.classification.as_persisted(),
            preview.rationale.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    for (ordinal, entry) in preview.changes.iter().enumerate() {
        tx.execute(
            "INSERT INTO prepared_rebuild_managed_projections_changes (prepared_intent_id,ordinal,relative_path,record_kind,record_id,change_kind,classification,expected_final_file_sha256) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![
                prepared.id().as_str(),
                i64::try_from(ordinal).map_err(|_| storage_error(context))?,
                entry.relative_path.as_str(),
                entry.record_kind.as_persisted(),
                entry.record_id.as_str(),
                entry.change.as_persisted(),
                entry.classification.as_persisted(),
                entry.expected_final_file_sha256.as_deref(),
            ],
        )
        .map_err(|_| storage_error(context))?;
    }
    Ok(())
}

fn persist_prepare_command_echo(
    tx: &Transaction<'_>,
    command: &PrepareRebuildManagedProjections,
    context: &OperationContext,
) -> Result<(), DomainError> {
    tx.execute(
        "INSERT INTO rebuild_managed_projections_prepare_commands (idempotency_id,ledger_schema_version,ledger_revision,published_baseline_digest,estimated_duration_millis,rationale) VALUES (?1,?2,?3,?4,?5,?6)",
        rusqlite::params![
            context.idempotency_id.as_str(),
            i64::from(command.ledger_schema_version),
            i64::try_from(command.ledger_revision).map_err(|_| storage_error(context))?,
            command.published_baseline_digest.as_str(),
            i64::try_from(command.estimated_duration_millis)
                .map_err(|_| storage_error(context))?,
            command.rationale.as_str(),
        ],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

/// Contiguous from zero, matching the trigger the V41 schema installs --
/// `COALESCE(MAX(..)+1, 0)`, not `1`, which is the off-by-one V40 shipped
/// and had to repair.
fn next_prepare_ordinal(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<i64, DomainError> {
    tx.query_row(
        "SELECT COALESCE(MAX(operation_ordinal)+1,0) FROM rebuild_managed_projections_prepare_replay_operations",
        [],
        |row| row.get(0),
    )
    .map_err(|_| storage_error(context))
}

fn rebuild_domain_error(
    error: &ManagedProjectionRebuildError,
    context: &OperationContext,
) -> DomainError {
    let (code, key) = match error {
        ManagedProjectionRebuildError::EmptyChangeSet => (
            ErrorCode::ValidationInvalidField,
            "projection.rebuild_empty_change_set",
        ),
        ManagedProjectionRebuildError::ChangesNotCanonical => (
            ErrorCode::ValidationInvalidField,
            "projection.rebuild_changes_not_canonical",
        ),
        ManagedProjectionRebuildError::DigestMismatch => (
            ErrorCode::SecurityPreviewExpiredOrChanged,
            "projection.rebuild_digest_mismatch",
        ),
        ManagedProjectionRebuildError::Expired => (
            ErrorCode::SecurityPreviewExpiredOrChanged,
            "projection.rebuild_preview_expired",
        ),
        ManagedProjectionRebuildError::PreviewChanged => (
            ErrorCode::SecurityPreviewExpiredOrChanged,
            "projection.rebuild_preview_changed",
        ),
        ManagedProjectionRebuildError::UnauthorizedActor => (
            ErrorCode::SecurityPolicyDenied,
            "projection.rebuild_unauthorized_actor",
        ),
        ManagedProjectionRebuildError::MissingConfirmation => (
            ErrorCode::SecurityPolicyDenied,
            "projection.rebuild_missing_confirmation",
        ),
        ManagedProjectionRebuildError::PreparedIntentMismatch => (
            ErrorCode::DomainConflict,
            "projection.rebuild_prepared_intent_mismatch",
        ),
        // Denied as policy, not as a validation slip: ADR 0007 requires
        // work beyond the ceiling to be escalated to H2a for approval
        // rather than retried as automation.
        ManagedProjectionRebuildError::H1AutoNotPermitted => (
            ErrorCode::SecurityPolicyDenied,
            "projection.rebuild_h1_auto_not_permitted",
        ),
        ManagedProjectionRebuildError::PublicationInFlight => (
            ErrorCode::DomainConflict,
            "projection.rebuild_publication_in_flight",
        ),
    };
    DomainError::new(
        code,
        MessageKey::parse(key).unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn storage_error(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::PlatformInternal,
        MessageKey::parse("projection.persistence_failed").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        true,
    )
}

/// Whether any publication is still `publishing` or `cancelling`.
///
/// Read inside the same `BEGIN IMMEDIATE` transaction that would open the
/// next one, so two openers serialize on SQLite's writer lock and the
/// check cannot race the insert. A schema-level UNIQUE partial index was
/// considered and deliberately not added yet: a Ledger that already holds
/// two stuck operations from earlier crashes would then fail to migrate,
/// and locking the user out is not recovery. The index can follow once
/// crash recovery for interrupted publications can resolve stuck rows.
fn publication_in_flight(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<bool, DomainError> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM rebuild_managed_projections_execute_replay_operations WHERE status IN ('publishing','cancelling'))",
        [],
        |row| row.get::<_, bool>(0),
    )
    .map_err(|_| storage_error(context))
}

/// Marks the projection head `out_of_sync`, keeping whichever generation
/// was last verified as the diff baseline.
///
/// Called when a publication *opens*, not only when it fails. ADR 0007
/// writes files outside any Ledger transaction, so from the moment an
/// operation opens the disk may differ from the last verified manifest;
/// "Synchronized" is an integrity result and cannot be claimed while that
/// is true. Before this, a crash between opening and completing left the
/// head `verified` against a partly rewritten tree, and because H1-Auto
/// gates only on the head, automation could start a second publication on
/// top of it.
fn mark_head_out_of_sync(
    tx: &Transaction<'_>,
    now: UtcTimestamp,
    context: &OperationContext,
) -> Result<(), DomainError> {
    tx.execute(
        "INSERT INTO projection_manifest_head (singleton,manifest_id,integrity_state,updated_at) VALUES (1,NULL,'out_of_sync',?1) ON CONFLICT(singleton) DO UPDATE SET integrity_state='out_of_sync',updated_at=excluded.updated_at",
        rusqlite::params![now.unix_millis()],
    )
    .map_err(|_| storage_error(context))?;
    Ok(())
}

fn idempotency_conflict(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainIdempotencyConflict,
        MessageKey::parse("projection.idempotency_conflict").unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn persist_operation_items(
    tx: &Transaction<'_>,
    context: &OperationContext,
    changes: &[ManagedProjectionRebuildChangeEntry],
) -> Result<(), DomainError> {
    for (ordinal, entry) in changes.iter().enumerate() {
        tx.execute(
            "INSERT INTO rebuild_managed_projections_operation_items (idempotency_id,ordinal,relative_path,change_kind,state) VALUES (?1,?2,?3,?4,'untouched')",
            rusqlite::params![
                context.idempotency_id.as_str(),
                i64::try_from(ordinal).map_err(|_| storage_error(context))?,
                entry.relative_path.as_str(),
                entry.change.as_persisted(),
            ],
        )
        .map_err(|_| storage_error(context))?;
    }
    Ok(())
}

fn existing_execute_operation(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<bool, DomainError> {
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM rebuild_managed_projections_execute_replay_operations WHERE idempotency_id=?1)",
        [context.idempotency_id.as_str()],
        |row| row.get::<_, i64>(0),
    )
    .map_err(|_| storage_error(context))
    .map(|flag| flag != 0)
}

/// A terminal report that does not account for every planned path exactly
/// once. Refused rather than partially applied: an effect scope derived
/// from a subset of the plan is a claim about the report, not about what
/// the operation did.
fn incomplete_report(context: &OperationContext) -> DomainError {
    DomainError::new(
        ErrorCode::DomainConflict,
        MessageKey::parse("projection.rebuild_incomplete_report")
            .unwrap_or_else(|_| unreachable!()),
        context.correlation_id.clone(),
        false,
    )
}

fn planned_item_count(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<usize, DomainError> {
    let count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM rebuild_managed_projections_operation_items WHERE idempotency_id=?1",
            [context.idempotency_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|_| storage_error(context))?;
    usize::try_from(count).map_err(|_| storage_error(context))
}

/// Whether the managed projection currently has a verified head -- the
/// synchronized baseline H1-Auto automation is only ever permitted to start
/// from. Absent head (nothing ever published) and `out_of_sync` both count
/// as not verified.
fn projection_head_is_verified(
    tx: &Transaction<'_>,
    context: &OperationContext,
) -> Result<bool, DomainError> {
    Ok(tx
        .query_row(
            "SELECT integrity_state FROM projection_manifest_head WHERE singleton=1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| storage_error(context))?
        .is_some_and(|state| state == "verified"))
}
