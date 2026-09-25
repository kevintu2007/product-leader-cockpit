/**
 * English words for every `messageKey` the host can put in a safe error
 * envelope, plus the field names and next steps an envelope can name.
 *
 * `safeErrors.test.ts` reads the Rust sources: every dotted string literal
 * there must be one of these keys or be listed as not a message. A key the
 * host gains without words here fails the build instead of reaching a
 * person as the generic error.
 *
 * Several keys differ only in which record they are about (Portfolio,
 * Product, Roadmap, KPI, ...). Each still has its own sentence, because the
 * record's name belongs in it and word order differs by language.
 */
export const SAFE_ERRORS_EN = {
  // The desktop host.
  "safeError.desktop.snapshot_unavailable": "The Product Ledger snapshot can't be read right now.",
  "safeError.desktop.snapshot_invalid":
    "The Product Ledger data failed its consistency check, so nothing was written on top of it.",
  "safeError.desktop.product_not_found": "This Product can't be found.",
  "safeError.desktop.product_detail_unattributable":
    "Part of this Product's detail has no traceable source, so it isn't shown.",
  "safeError.desktop.invalid_argument": "The submitted data was malformed. Nothing was written.",
  "safeError.desktop.action_not_found": "This Action can't be found.",
  "safeError.desktop.host_id_source_failed":
    "The app couldn't generate the identifier this operation needs.",
  "safeError.desktop.host_identifier_failed":
    "The app couldn't generate the identifier this operation needs.",
  "safeError.desktop.unsupported_preview":
    "The app prepared a kind of preview this screen can't show, so it was not shown.",
  "safeError.desktop.preview_already_consumed":
    "This preview was already approved or rejected. Prepare it again to continue.",
  "safeError.desktop.evidence_not_found": "This Evidence can't be found.",
  "safeError.desktop.evidence_already_pinned": "This Evidence's fingerprint is already pinned.",
  "safeError.desktop.evidence_version_conflict":
    "This Evidence changed after you read it. Reload and try again.",
  "safeError.desktop.evidence_path_containment_failed":
    "This Evidence's file is not inside the Product Vault, so it wasn't read.",
  "safeError.desktop.evidence_source_not_observable":
    "This Evidence's file can't be read right now.",
  "safeError.desktop.vault_not_configured":
    "No Product Vault is set for this workspace yet. Choose one in Settings.",
  "safeError.desktop.vault_root_unavailable":
    "The Product Vault folder can't be reached right now.",
  "safeError.desktop.settings_unavailable": "Display settings can't be read or saved right now.",
  "safeError.desktop.backup_destination_unusable":
    "That folder can't hold backups. Choose an existing folder on this computer that you can write to, and not a shortcut or link.",
  "safeError.desktop.passphrase_too_short":
    "That passphrase is too short. Use at least 20 characters or six words.",
  "safeError.desktop.passphrase_too_repetitive":
    "That passphrase repeats itself too much. Use more varied words or characters.",
  "safeError.desktop.passphrase_too_long":
    "That passphrase is too long. Use at most 1,024 characters.",
  "safeError.desktop.passphrase_too_common":
    "That passphrase is made of very common passwords. Use words that aren't common passwords.",
  "safeError.desktop.credential_unavailable":
    "Windows Credential Manager couldn't store or read the passphrase right now.",
  "safeError.desktop.random_unavailable":
    "The app couldn't get the randomness it needs to make a passphrase.",

  // The Ledger itself.
  "safeError.ledger.open.unclaimed_database":
    "This Product Ledger file wasn't created by this app.",
  "safeError.ledger.open.wrong_application": "This file is not a Product Mission Control Ledger.",
  "safeError.ledger.open.future_schema":
    "This Product Ledger comes from a newer version of the app and can't be opened by this one.",
  "safeError.ledger.open.unsupported_schema":
    "This version of the app doesn't support this Product Ledger's schema version.",
  "safeError.ledger.open.invalid_metadata": "The Product Ledger's metadata is inconsistent.",
  "safeError.ledger.open.corrupt_database": "The Product Ledger file is damaged.",
  "safeError.ledger.open.policy_violation":
    "The Product Ledger connection settings don't meet the security policy.",
  "safeError.ledger.open.busy": "The Product Ledger is busy.",
  "safeError.ledger.open.storage_unavailable": "The storage device is unavailable.",
  "safeError.ledger.transaction.revision_conflict":
    "The data changed after you read it. Reload and try again.",
  "safeError.ledger.transaction.incompatible_ledger":
    "The Product Ledger isn't compatible with this version of the app.",
  "safeError.ledger.transaction.busy": "The Product Ledger is busy.",
  "safeError.ledger.transaction.commit_failed":
    "The write didn't finish, and whether it was saved can't be confirmed.",
  "safeError.ledger.commit_failed":
    "The write didn't finish, and whether it was saved can't be confirmed.",
  "safeError.ledger.persistence_failed": "The Product Ledger couldn't save this.",
  "safeError.ledger.idempotency_conflict":
    "This request identifier was already used for a different operation.",
  "safeError.audit.failed":
    "The audit record for this operation couldn't be written, so nothing changed.",
  "safeError.version.overflow": "This record has reached the highest version it can hold.",
  "safeError.classification.lowering_requires_governed_intent":
    "Lowering a classification needs its own reviewed and approved step.",

  // Action Requests and Actions.
  "safeError.action.not_found": "This Action Request or Action can't be found.",
  "safeError.action_request.already_exists":
    "An Action Request with this identifier already exists.",
  "safeError.action.idempotency_conflict":
    "This request identifier was already used for a different operation.",
  "safeError.action.approval_denied":
    "This isn't authorized, was refused by policy, or the Evidence or Judgment doesn't support it.",
  "safeError.action.preview_expired_or_changed":
    "The preview expired, or changed before you confirmed it. Prepare it again.",
  "safeError.action.internal": "The app hit an internal error while handling this.",
  "safeError.action.validation_failed":
    "A required field is missing (for example an owner or a due date).",
  "safeError.action.domain_conflict":
    "This conflicts with the record's current state or version. Reload and try again.",
  "safeError.action.classification_lowering_not_a_lowering":
    "The proposed classification is not lower than the current one.",
  "safeError.action.completion_evidence_already_linked":
    "This Evidence is already linked as completion Evidence for this Action.",

  // Decision Requests and Decisions.
  "safeError.decision.not_found": "This Decision Request or Decision can't be found.",
  "safeError.decision.already_exists": "A Decision with this identifier already exists.",
  "safeError.decision.conflict":
    "This conflicts with the Decision Request's current state or version. Reload and try again.",
  "safeError.decision.request_transition_conflict":
    "The Decision Request's state doesn't allow this now. Reload and try again.",
  "safeError.decision.approval_denied":
    "This isn't authorized, or the Evidence or Judgment doesn't support it.",
  "safeError.decision.preview_changed":
    "The preview expired, or changed before you confirmed it. Prepare it again.",
  "safeError.decision.internal": "The app hit an internal error while handling this.",
  "safeError.decision.classification_lowering_not_a_lowering":
    "The proposed classification is not lower than the current one.",

  // Risks.
  "safeError.risk.not_found": "This Risk can't be found.",
  "safeError.risk.owner_not_found": "The Stakeholder named as owner can't be found.",
  "safeError.risk.next_review_before_epoch": "The next review date can't be before 1970.",
  "safeError.risk.already_exists": "A Risk with this identifier already exists.",
  "safeError.risk.classification": "Choose a classification for this Risk.",
  "safeError.risk.accepted_fields_required":
    "Accepting or transferring a Risk needs its rationale and review date.",
  "safeError.risk.conflict": "This conflicts with the Risk's current state. Reload and try again.",
  "safeError.risk.stale_or_illegal":
    "The Risk changed, or its state doesn't allow this. Reload and try again.",
  "safeError.risk.idempotency_conflict":
    "This request identifier was already used for a different operation.",
  "safeError.risk.preview_changed":
    "The preview expired, or changed before you confirmed it. Prepare it again.",
  "safeError.risk.security_denied": "This isn't authorized for this Risk.",
  "safeError.risk.evidence_unavailable": "The Evidence this needs can't be read right now.",
  "safeError.risk.infrastructure": "The app hit an internal error while handling this Risk.",

  // Issues.
  "safeError.issue.not_found": "This Issue can't be found.",
  "safeError.issue.exists": "An Issue with this identifier already exists.",
  "safeError.issue.classification_required": "Choose a classification for this Issue.",
  "safeError.issue.stale_or_illegal":
    "The Issue changed, or its state doesn't allow this. Reload and try again.",
  "safeError.issue.invalid_intent": "This Issue's state doesn't allow this operation.",
  "safeError.issue.prepared_operation_mismatch":
    "This preview was prepared for a different operation. Prepare it again.",
  "safeError.issue.preview_changed":
    "The preview expired, or changed before you confirmed it. Prepare it again.",
  "safeError.issue.idempotency_conflict":
    "This request identifier was already used for a different operation.",
  "safeError.issue.recurrence_not_supported": "This Issue can't be recorded as a recurrence.",
  "safeError.issue.security_denied": "This isn't authorized for this Issue.",
  "safeError.issue.infrastructure": "The app hit an internal error while handling this Issue.",
  "safeError.issue.classification_lowering_not_a_lowering":
    "The proposed classification is not lower than the current one.",

  // Evidence.
  "safeError.evidence.not_found": "This Evidence can't be found.",
  "safeError.evidence_reference.not_found": "This Evidence can't be found.",
  "safeError.evidence.already_exists": "Evidence with this identifier already exists.",
  "safeError.evidence.path_already_referenced":
    "Another Evidence reference for this file was just created. Choose the file again to see it.",
  "safeError.evidence.idempotency_conflict":
    "This request identifier was already used for a different operation.",
  "safeError.evidence.persistence_failed": "The Product Ledger couldn't save this Evidence.",
  "safeError.evidence.pin_fingerprint_already_pinned":
    "This Evidence's fingerprint is already pinned.",
  "safeError.evidence.pin_path_mismatch":
    "The file read doesn't match this Evidence's recorded path, so no fingerprint was pinned.",
  "safeError.evidence.relocation_path_unchanged":
    "The new location is the same as the current one.",
  "safeError.evidence.relocation_path_mismatch":
    "The file read doesn't match the new location you gave.",
  "safeError.evidence.relocation_fingerprint_mismatch":
    "The file at the new location isn't the same file: its fingerprint differs.",
  "safeError.evidence.relocation_fingerprint_unpinned":
    "This Evidence has no pinned fingerprint, so a move can't be confirmed to be the same file.",
  "safeError.evidence.supersession_source_mismatch":
    "This replacement was prepared for different Evidence.",
  "safeError.evidence.supersession_source_already_superseded":
    "This Evidence has already been replaced.",
  "safeError.evidence.supersession_replacement_is_source": "Evidence can't replace itself.",
  "safeError.evidence.supersession_not_a_genuine_replacement":
    "The replacement is the same file as the Evidence it would replace.",
  "safeError.evidence.supersession_lowers_classification":
    "A replacement can't have a lower classification than the Evidence it replaces.",
  "safeError.evidence.supersession_unclassified_replacement":
    "Choose a classification for the replacement Evidence.",
  "safeError.evidence.supersession_missing_confirmation":
    "Replacing Evidence needs your confirmation.",
  "safeError.evidence.supersession_unauthorized_actor": "This isn't authorized for this Evidence.",
  "safeError.evidence.supersession_prepared_intent_mismatch":
    "This approval is for a different prepared replacement.",
  "safeError.evidence.supersession_digest_mismatch":
    "The replacement changed after it was prepared. Prepare it again.",
  "safeError.evidence.supersession_preview_changed":
    "The preview changed before you confirmed it. Prepare it again.",
  "safeError.evidence.supersession_expired": "The preview expired. Prepare it again.",

  // Portfolio records.
  "safeError.portfolio.not_found": "This Portfolio can't be found.",
  "safeError.portfolio.already_exists": "A Portfolio with this identifier already exists.",
  "safeError.portfolio.stale_version":
    "This Portfolio changed after you read it. Reload and try again.",
  "safeError.portfolio.version_exhausted":
    "This Portfolio has reached the highest version it can hold.",
  "safeError.portfolio.idempotency_conflict":
    "This request identifier was already used for a different operation.",
  "safeError.portfolio.repository_unavailable": "Portfolio records can't be read right now.",
  "safeError.portfolio.fan_out_state_invalid":
    "The app found inconsistent Portfolio state and stopped before writing.",
  "safeError.portfolio.operation_ordinal_exhausted":
    "The app can't record any more operations of this kind.",
  "safeError.portfolio.audit_id_unavailable":
    "The app couldn't generate the audit identifier this operation needs.",
  "safeError.portfolio.prepared_intent_id_unavailable":
    "The app couldn't generate the identifier this preview needs.",
  "safeError.portfolio.approval_receipt_id_unavailable":
    "The app couldn't generate the identifier this approval needs.",
  "safeError.portfolio.classification_lowering_invalid":
    "This Portfolio's classification can't be lowered this way.",
  "safeError.portfolio.classification_lowering_not_a_lowering":
    "The proposed classification is not lower than this Portfolio's current one.",
  "safeError.portfolio.classification_lowering_preview_changed":
    "This Portfolio changed after the preview was prepared. Prepare it again.",
  "safeError.portfolio.classification_lowering_approval_mismatch":
    "This approval is for a different preview.",
  "safeError.portfolio.classification_lowering_approval_denied":
    "Lowering this Portfolio's classification wasn't approved.",

  "safeError.product.not_found": "This Product can't be found.",
  "safeError.product.already_exists": "A Product with this identifier already exists.",
  "safeError.product.stale_version":
    "This Product changed after you read it. Reload and try again.",
  "safeError.product.version_exhausted":
    "This Product has reached the highest version it can hold.",
  "safeError.product.prepared_intent_id_unavailable":
    "The app couldn't generate the identifier this preview needs.",
  "safeError.product.approval_receipt_id_unavailable":
    "The app couldn't generate the identifier this approval needs.",
  "safeError.product.classification_lowering_invalid":
    "This Product's classification can't be lowered this way.",
  "safeError.product.classification_lowering_not_a_lowering":
    "The proposed classification is not lower than this Product's current one.",
  "safeError.product.classification_lowering_preview_changed":
    "This Product changed after the preview was prepared. Prepare it again.",
  "safeError.product.classification_lowering_approval_mismatch":
    "This approval is for a different preview.",
  "safeError.product.classification_lowering_approval_denied":
    "Lowering this Product's classification wasn't approved.",

  "safeError.roadmap.not_found": "This Roadmap can't be found.",
  "safeError.roadmap.already_exists": "A Roadmap with this identifier already exists.",
  "safeError.roadmap.stale_version":
    "This Roadmap changed after you read it. Reload and try again.",
  "safeError.roadmap.version_exhausted":
    "This Roadmap has reached the highest version it can hold.",
  "safeError.roadmap.prepared_intent_id_unavailable":
    "The app couldn't generate the identifier this preview needs.",
  "safeError.roadmap.approval_receipt_id_unavailable":
    "The app couldn't generate the identifier this approval needs.",
  "safeError.roadmap.classification_lowering_invalid":
    "This Roadmap's classification can't be lowered this way.",
  "safeError.roadmap.classification_lowering_not_a_lowering":
    "The proposed classification is not lower than this Roadmap's current one.",
  "safeError.roadmap.classification_lowering_preview_changed":
    "This Roadmap changed after the preview was prepared. Prepare it again.",
  "safeError.roadmap.classification_lowering_approval_mismatch":
    "This approval is for a different preview.",
  "safeError.roadmap.classification_lowering_approval_denied":
    "Lowering this Roadmap's classification wasn't approved.",

  "safeError.kpi.not_found": "This KPI can't be found.",
  "safeError.kpi.already_exists": "A KPI with this identifier already exists.",
  "safeError.kpi.stale_version": "This KPI changed after you read it. Reload and try again.",
  "safeError.kpi.version_exhausted": "This KPI has reached the highest version it can hold.",
  "safeError.kpi.prepared_intent_id_unavailable":
    "The app couldn't generate the identifier this preview needs.",
  "safeError.kpi.approval_receipt_id_unavailable":
    "The app couldn't generate the identifier this approval needs.",
  "safeError.kpi.classification_lowering_invalid":
    "This KPI's classification can't be lowered this way.",
  "safeError.kpi.classification_lowering_not_a_lowering":
    "The proposed classification is not lower than this KPI's current one.",
  "safeError.kpi.classification_lowering_preview_changed":
    "This KPI changed after the preview was prepared. Prepare it again.",
  "safeError.kpi.classification_lowering_approval_mismatch":
    "This approval is for a different preview.",
  "safeError.kpi.classification_lowering_approval_denied":
    "Lowering this KPI's classification wasn't approved.",

  "safeError.kpi.observation.not_found": "This KPI observation can't be found.",
  "safeError.kpi.observation.already_exists":
    "A KPI observation with this identifier already exists.",
  "safeError.kpi.observation.stale_version":
    "This KPI observation changed after you read it. Reload and try again.",
  "safeError.kpi.observation.version_exhausted":
    "This KPI observation has reached the highest version it can hold.",
  "safeError.kpi.observation.prepared_intent_id_unavailable":
    "The app couldn't generate the identifier this preview needs.",
  "safeError.kpi.observation.approval_receipt_id_unavailable":
    "The app couldn't generate the identifier this approval needs.",
  "safeError.kpi.observation.classification_lowering_invalid":
    "This KPI observation's classification can't be lowered this way.",
  "safeError.kpi.observation.classification_lowering_not_a_lowering":
    "The proposed classification is not lower than this KPI observation's current one.",
  "safeError.kpi.observation.classification_lowering_preview_changed":
    "This KPI observation changed after the preview was prepared. Prepare it again.",
  "safeError.kpi.observation.classification_lowering_approval_mismatch":
    "This approval is for a different preview.",
  "safeError.kpi.observation.classification_lowering_approval_denied":
    "Lowering this KPI observation's classification wasn't approved.",

  // Initiatives, Projects and Milestones.
  "safeError.delivery.not_found": "This Initiative, Project or Milestone can't be found.",
  "safeError.delivery.already_exists": "A record with this identifier already exists.",
  "safeError.delivery.stale_version":
    "This record changed after you read it. Reload and try again.",
  "safeError.delivery.version_exhausted":
    "This record has reached the highest version it can hold.",
  "safeError.delivery.conflict":
    "This conflicts with the record's current state. Reload and try again.",
  "safeError.delivery.idempotency_conflict":
    "This request identifier was already used for a different operation.",
  "safeError.delivery.persistence_failed": "The Product Ledger couldn't save this.",
  "safeError.delivery.invalid_period": "The start date is after the end date.",
  "safeError.delivery.preview_changed":
    "The preview expired, or changed before you confirmed it. Prepare it again.",
  "safeError.delivery.validation.invalid_field": "A field has a value that isn't allowed.",
  "safeError.delivery.validation.invalid_text": "A text field is empty or too long.",
  "safeError.delivery.validation.invalid_period": "The start date is after the end date.",
  "safeError.delivery.classification.lowering_denied":
    "Lowering this classification isn't allowed.",
  "safeError.delivery.classification_lowering_invalid":
    "This record's classification can't be lowered this way.",
  "safeError.delivery.classification_lowering_not_a_lowering":
    "The proposed classification is not lower than the current one.",
  "safeError.delivery.classification_lowering_preview_changed":
    "This record changed after the preview was prepared. Prepare it again.",

  // Relationships.
  "safeError.relationship.not_found": "This relationship can't be found.",
  "safeError.relationship.conflict":
    "This conflicts with the relationship's current state. Reload and try again.",
  "safeError.relationship.idempotency_conflict":
    "This request identifier was already used for a different operation.",
  "safeError.relationship.persistence_failed":
    "The Product Ledger couldn't save this relationship.",
  "safeError.relationship.milestone_subject_not_supported":
    "A Milestone can't be the subject of this kind of relationship.",
  "safeError.relationship.classification.unclassified_or_lowering_denied":
    "Choose a classification that isn't lower than the records it connects.",
  "safeError.relationship.removal.authorization_denied":
    "Removing this relationship isn't authorized.",
  "safeError.relationship.removal.policy_denied":
    "Policy doesn't allow removing this relationship.",
  "safeError.relationship.removal.confirmation_mismatch":
    "The confirmation doesn't match. Type it exactly as shown.",
  "safeError.relationship.removal.preview_expired_or_changed":
    "The preview expired, or changed before you confirmed it. Prepare it again.",
  "safeError.relationship.removal.too_late_to_cancel":
    "This removal has already been approved, so it can't be cancelled.",

  // Managed projections.
  "safeError.projection.idempotency_conflict":
    "This request identifier was already used for a different operation.",
  "safeError.projection.persistence_failed": "The Product Ledger couldn't save this projection.",
  "safeError.projection.rebuild_operation_not_found": "This projection rebuild can't be found.",
  "safeError.projection.rebuild_operation_terminal":
    "This projection rebuild has already finished.",
  "safeError.projection.rebuild_prepared_intent_not_found":
    "The prepared rebuild can't be found. Prepare it again.",
  "safeError.projection.rebuild_prepared_intent_consumed":
    "This prepared rebuild was already used. Prepare it again.",
  "safeError.projection.rebuild_prepared_intent_mismatch":
    "This approval is for a different prepared rebuild.",
  "safeError.projection.rebuild_preview_changed":
    "The rebuild preview changed before you confirmed it. Prepare it again.",
  "safeError.projection.rebuild_preview_expired": "The rebuild preview expired. Prepare it again.",
  "safeError.projection.rebuild_digest_mismatch":
    "The rebuild changed after it was prepared. Prepare it again.",
  "safeError.projection.rebuild_missing_confirmation":
    "A projection rebuild needs your confirmation.",
  "safeError.projection.rebuild_unauthorized_actor": "This isn't authorized for projections.",
  "safeError.projection.rebuild_h1_auto_not_permitted":
    "This rebuild needs a review; it can't run automatically.",
  "safeError.projection.rebuild_publication_in_flight":
    "Another projection publication is still running. Try again when it finishes.",
  "safeError.projection.rebuild_empty_change_set": "There is nothing to rebuild.",
  "safeError.projection.rebuild_changes_not_canonical":
    "The planned changes aren't in the expected form, so nothing was rebuilt.",
  "safeError.projection.rebuild_unplanned_item":
    "The rebuild found an item that wasn't in its plan, so it stopped.",
  "safeError.projection.rebuild_incomplete_report":
    "The rebuild report is incomplete, so it can't be confirmed.",

  // Field names an envelope can point at.
  "field.delivery.name": "name",
  "field.initiative.defined_outcome": "defined outcome",
  "field.milestone.verification_criteria": "verification criteria",
  "field.project.time_range": "time range",

  // Next steps an envelope can suggest.
  "nextStep.issue.prepare_resolve": "Prepare a resolution.",
  "nextStep.issue.prepare_close_or_reopen": "Prepare to close or reopen it.",
  "nextStep.issue.no_transition": "A closed Issue has no further steps.",
  "nextStep.issue.refresh_and_reprepare": "Reload, then prepare it again.",
  "nextStep.risk.update_response_or_prepare_transition":
    "Update the response, or prepare a transition.",
  "nextStep.risk.no_transition": "This Risk has no further steps.",
  "nextStep.risk.refresh_and_reprepare": "Reload, then prepare it again.",
  "safeError.desktop.backup_due": "A backup is due. Back up first, then try again.",
  "safeError.desktop.backup_running": "A backup is running. Try again when it finishes.",
  "safeError.desktop.backup_destination_not_set": "Choose a backup folder first.",
  "safeError.desktop.backup_destination_unavailable":
    "The backup folder can't be reached. Plug in the drive or choose another folder.",
  "safeError.desktop.backup_passphrase_required": "Set up the recovery passphrase first.",
  "safeError.desktop.backup_verification_failed":
    "The backup didn't pass its check, so it wasn't kept. Try again.",
  "safeError.desktop.backup_failed": "The backup didn't complete. Try again.",
  "safeError.desktop.restore_ledger_locked":
    "Another program has the current Ledger files open. Close it and try again. Nothing was changed.",
  "safeError.desktop.restore_preservation_failed":
    "PMC could not preserve and verify the current Ledger files. Nothing was changed. Restore cannot continue.",
  "safeError.desktop.restore_state_unreadable":
    "PMC cannot read the restore state record, so it will not restore. Nothing was changed.",
  "safeError.desktop.restore_recovery_backup_missing":
    "The recovery backup is no longer in the backup folder, or it was altered. Choose another backup.",
} as const;
