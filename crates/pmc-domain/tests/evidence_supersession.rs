use pmc_domain::audit::AuditActor;
use pmc_domain::classification::DataClassification;
use pmc_domain::evidence::{
    execute_supersede_evidence_reference, prepare_supersede_evidence_reference,
    EvidenceFingerprint, EvidenceLinkTarget, EvidenceReferenceRecord, EvidenceSupersessionApproval,
    EvidenceSupersessionError, EvidenceSupersessionLinkSnapshot,
    EvidenceSupersessionSourceSnapshot, FingerprintAlgorithm, PrepareSupersedeEvidenceReference,
    VaultRelativePath,
};
use pmc_domain::identity::{
    AggregateVersion, CorrelationId, EvidenceReferenceId, IdempotencyId, KpiId, PreparedIntentId,
};
use pmc_domain::provenance::Provenance;
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{EvidenceVerification, IntegrityDigest};

const PINNED_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const REPLACEMENT_DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn source_record(
    fingerprint: Option<&str>,
    classification: DataClassification,
) -> EvidenceReferenceRecord {
    EvidenceReferenceRecord {
        id: EvidenceReferenceId::parse("evidence-source").unwrap(),
        vault_path: VaultRelativePath::parse("Research/notes.md").unwrap(),
        fingerprint: fingerprint.map(|digest| {
            EvidenceFingerprint::new(
                FingerprintAlgorithm::Sha256,
                IntegrityDigest::parse(digest).unwrap(),
            )
        }),
        verification: EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(1_000),
            integrity_digest: IntegrityDigest::parse(fingerprint.unwrap_or(PINNED_DIGEST)).unwrap(),
        },
        classification,
        provenance: Provenance::UserEntered,
        version: AggregateVersion::initial(),
        created_at: UtcTimestamp::from_unix_millis(1_000),
        updated_at: UtcTimestamp::from_unix_millis(1_000),
    }
}

fn one_link() -> EvidenceSupersessionLinkSnapshot {
    EvidenceSupersessionLinkSnapshot {
        target: EvidenceLinkTarget::Kpi(KpiId::parse("kpi-1").unwrap()),
        target_version: AggregateVersion::initial(),
        target_classification: DataClassification::Internal,
        link_classification: DataClassification::Internal,
        linked_at: UtcTimestamp::from_unix_millis(2_000),
    }
}

fn prepare_intent(
    source_id: EvidenceReferenceId,
    expected_source_version: AggregateVersion,
    replacement_classification: DataClassification,
) -> PrepareSupersedeEvidenceReference {
    PrepareSupersedeEvidenceReference {
        source_id,
        expected_source_version,
        replacement_id: EvidenceReferenceId::parse("evidence-replacement").unwrap(),
        replacement_vault_path: VaultRelativePath::parse("Research/notes-v2.md").unwrap(),
        replacement_fingerprint: EvidenceFingerprint::new(
            FingerprintAlgorithm::Sha256,
            IntegrityDigest::parse(REPLACEMENT_DIGEST).unwrap(),
        ),
        replacement_observed_at: UtcTimestamp::from_unix_millis(5_000),
        replacement_classification,
        replacement_provenance: Provenance::UserEntered,
        rationale: pmc_domain::work_management::WorkManagementRationale::parse(
            "File was re-exported after a content rewrite",
        )
        .unwrap(),
        context: pmc_domain::evidence::OperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-supersede-prepare-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-supersede-correlation-1").unwrap(),
        },
    }
}

#[test]
fn prepare_rejects_a_replacement_fingerprint_equal_to_the_source() {
    let source = source_record(Some(PINNED_DIGEST), DataClassification::Internal);
    let snapshot = EvidenceSupersessionSourceSnapshot {
        record: source.clone(),
        links: vec![],
    };
    let mut intent = prepare_intent(
        source.id.clone(),
        source.version,
        DataClassification::Internal,
    );
    intent.replacement_fingerprint = EvidenceFingerprint::new(
        FingerprintAlgorithm::Sha256,
        IntegrityDigest::parse(PINNED_DIGEST).unwrap(),
    );

    let result = prepare_supersede_evidence_reference(
        &intent,
        PreparedIntentId::parse("prepared-1").unwrap(),
        &snapshot,
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert_eq!(
        result.unwrap_err(),
        EvidenceSupersessionError::NotAGenuineReplacement
    );
}

#[test]
fn prepare_allows_superseding_an_unpinned_source() {
    let source = source_record(None, DataClassification::Internal);
    let snapshot = EvidenceSupersessionSourceSnapshot {
        record: source.clone(),
        links: vec![],
    };
    let intent = prepare_intent(
        source.id.clone(),
        source.version,
        DataClassification::Internal,
    );

    let prepared = prepare_supersede_evidence_reference(
        &intent,
        PreparedIntentId::parse("prepared-1").unwrap(),
        &snapshot,
        UtcTimestamp::from_unix_millis(5_000),
    )
    .unwrap();
    assert_eq!(prepared.preview().source_fingerprint, None);
}

#[test]
fn prepare_rejects_an_unclassified_replacement() {
    let source = source_record(Some(PINNED_DIGEST), DataClassification::Internal);
    let snapshot = EvidenceSupersessionSourceSnapshot {
        record: source.clone(),
        links: vec![],
    };
    let intent = prepare_intent(
        source.id.clone(),
        source.version,
        DataClassification::Unclassified,
    );

    let result = prepare_supersede_evidence_reference(
        &intent,
        PreparedIntentId::parse("prepared-1").unwrap(),
        &snapshot,
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert_eq!(
        result.unwrap_err(),
        EvidenceSupersessionError::UnclassifiedReplacement
    );
}

#[test]
fn prepare_rejects_a_replacement_that_lowers_classification() {
    let source = source_record(Some(PINNED_DIGEST), DataClassification::Confidential);
    let snapshot = EvidenceSupersessionSourceSnapshot {
        record: source.clone(),
        links: vec![],
    };
    let intent = prepare_intent(
        source.id.clone(),
        source.version,
        DataClassification::Internal,
    );

    let result = prepare_supersede_evidence_reference(
        &intent,
        PreparedIntentId::parse("prepared-1").unwrap(),
        &snapshot,
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert_eq!(
        result.unwrap_err(),
        EvidenceSupersessionError::ReplacementLowersClassification
    );
}

#[test]
fn prepare_rejects_a_stale_expected_source_version() {
    let source = source_record(Some(PINNED_DIGEST), DataClassification::Internal);
    let snapshot = EvidenceSupersessionSourceSnapshot {
        record: source.clone(),
        links: vec![],
    };
    let intent = prepare_intent(
        source.id.clone(),
        AggregateVersion::new(2).unwrap(),
        DataClassification::Internal,
    );

    let result = prepare_supersede_evidence_reference(
        &intent,
        PreparedIntentId::parse("prepared-1").unwrap(),
        &snapshot,
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert_eq!(
        result.unwrap_err(),
        EvidenceSupersessionError::SourceMismatch
    );
}

#[test]
fn prepare_rejects_replacement_id_equal_to_source_id() {
    let source = source_record(Some(PINNED_DIGEST), DataClassification::Internal);
    let snapshot = EvidenceSupersessionSourceSnapshot {
        record: source.clone(),
        links: vec![],
    };
    let mut intent = prepare_intent(
        source.id.clone(),
        source.version,
        DataClassification::Internal,
    );
    intent.replacement_id = source.id.clone();

    let result = prepare_supersede_evidence_reference(
        &intent,
        PreparedIntentId::parse("prepared-1").unwrap(),
        &snapshot,
        UtcTimestamp::from_unix_millis(5_000),
    );
    assert_eq!(
        result.unwrap_err(),
        EvidenceSupersessionError::ReplacementIsSource
    );
}

#[test]
fn two_prepares_over_identical_inputs_produce_identical_digests() {
    let source = source_record(Some(PINNED_DIGEST), DataClassification::Internal);
    let snapshot = EvidenceSupersessionSourceSnapshot {
        record: source.clone(),
        links: vec![one_link()],
    };
    let intent = prepare_intent(
        source.id.clone(),
        source.version,
        DataClassification::Internal,
    );

    let first = prepare_supersede_evidence_reference(
        &intent,
        PreparedIntentId::parse("prepared-1").unwrap(),
        &snapshot,
        UtcTimestamp::from_unix_millis(5_000),
    )
    .unwrap();
    let second = prepare_supersede_evidence_reference(
        &intent,
        PreparedIntentId::parse("prepared-1").unwrap(),
        &snapshot,
        UtcTimestamp::from_unix_millis(5_000),
    )
    .unwrap();

    assert_eq!(
        first.payload_digest().as_str(),
        second.payload_digest().as_str()
    );
}

#[test]
fn execute_succeeds_and_produces_the_replacement_and_cloned_links() {
    let source = source_record(Some(PINNED_DIGEST), DataClassification::Internal);
    let snapshot = EvidenceSupersessionSourceSnapshot {
        record: source.clone(),
        links: vec![one_link()],
    };
    let intent = prepare_intent(
        source.id.clone(),
        source.version,
        DataClassification::Internal,
    );
    let prepared = prepare_supersede_evidence_reference(
        &intent,
        PreparedIntentId::parse("prepared-1").unwrap(),
        &snapshot,
        UtcTimestamp::from_unix_millis(5_000),
    )
    .unwrap();

    let approval = EvidenceSupersessionApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-supersede-execute-1").unwrap(),
        true,
    )
    .unwrap();

    let effects = execute_supersede_evidence_reference(
        &approval,
        &prepared,
        &snapshot,
        false,
        UtcTimestamp::from_unix_millis(6_000),
    )
    .unwrap();

    assert_eq!(effects.replacement.id.as_str(), "evidence-replacement");
    assert_eq!(effects.replacement.version.get(), 1);
    assert_eq!(
        effects.replacement.vault_path.as_str(),
        "Research/notes-v2.md"
    );
    assert_eq!(
        effects.replacement.fingerprint.unwrap().digest().as_str(),
        REPLACEMENT_DIGEST
    );
    assert_eq!(effects.replacement_links.len(), 1);
    assert_eq!(
        effects.replacement_links[0].evidence_id.as_str(),
        "evidence-replacement"
    );
    assert_eq!(effects.source_id.as_str(), "evidence-source");
    assert_eq!(effects.source_new_version.get(), 2);
}

#[test]
fn execute_rejects_a_changed_link_set_since_prepare() {
    let source = source_record(Some(PINNED_DIGEST), DataClassification::Internal);
    let snapshot_at_prepare = EvidenceSupersessionSourceSnapshot {
        record: source.clone(),
        links: vec![],
    };
    let intent = prepare_intent(
        source.id.clone(),
        source.version,
        DataClassification::Internal,
    );
    let prepared = prepare_supersede_evidence_reference(
        &intent,
        PreparedIntentId::parse("prepared-1").unwrap(),
        &snapshot_at_prepare,
        UtcTimestamp::from_unix_millis(5_000),
    )
    .unwrap();

    let approval = EvidenceSupersessionApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-supersede-execute-1").unwrap(),
        true,
    )
    .unwrap();

    // A link was added after prepare but before execute.
    let snapshot_at_execute = EvidenceSupersessionSourceSnapshot {
        record: source,
        links: vec![one_link()],
    };

    let result = execute_supersede_evidence_reference(
        &approval,
        &prepared,
        &snapshot_at_execute,
        false,
        UtcTimestamp::from_unix_millis(6_000),
    );
    assert_eq!(
        result.unwrap_err(),
        EvidenceSupersessionError::PreviewChanged
    );
}

#[test]
fn execute_rejects_a_mismatched_acknowledged_digest() {
    let source = source_record(Some(PINNED_DIGEST), DataClassification::Internal);
    let snapshot = EvidenceSupersessionSourceSnapshot {
        record: source.clone(),
        links: vec![],
    };
    let intent = prepare_intent(
        source.id.clone(),
        source.version,
        DataClassification::Internal,
    );
    let prepared = prepare_supersede_evidence_reference(
        &intent,
        PreparedIntentId::parse("prepared-1").unwrap(),
        &snapshot,
        UtcTimestamp::from_unix_millis(5_000),
    )
    .unwrap();

    let forged_digest =
        pmc_domain::evidence::EvidenceSupersessionPayloadDigest::from_persisted("f".repeat(64))
            .unwrap();
    let approval = EvidenceSupersessionApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        forged_digest,
        IdempotencyId::parse("synthetic-supersede-execute-1").unwrap(),
        true,
    )
    .unwrap();

    let result = execute_supersede_evidence_reference(
        &approval,
        &prepared,
        &snapshot,
        false,
        UtcTimestamp::from_unix_millis(6_000),
    );
    assert_eq!(
        result.unwrap_err(),
        EvidenceSupersessionError::DigestMismatch
    );
}

#[test]
fn execute_rejects_an_expired_prepared_intent() {
    let source = source_record(Some(PINNED_DIGEST), DataClassification::Internal);
    let snapshot = EvidenceSupersessionSourceSnapshot {
        record: source.clone(),
        links: vec![],
    };
    let intent = prepare_intent(
        source.id.clone(),
        source.version,
        DataClassification::Internal,
    );
    let prepared = prepare_supersede_evidence_reference(
        &intent,
        PreparedIntentId::parse("prepared-1").unwrap(),
        &snapshot,
        UtcTimestamp::from_unix_millis(5_000),
    )
    .unwrap();

    let approval = EvidenceSupersessionApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-supersede-execute-1").unwrap(),
        true,
    )
    .unwrap();

    let far_future = UtcTimestamp::from_unix_millis(5_000 + 300_000 + 1);
    let result =
        execute_supersede_evidence_reference(&approval, &prepared, &snapshot, false, far_future);
    assert_eq!(result.unwrap_err(), EvidenceSupersessionError::Expired);
}

#[test]
fn execute_rejects_an_already_superseded_source() {
    let source = source_record(Some(PINNED_DIGEST), DataClassification::Internal);
    let snapshot = EvidenceSupersessionSourceSnapshot {
        record: source.clone(),
        links: vec![],
    };
    let intent = prepare_intent(
        source.id.clone(),
        source.version,
        DataClassification::Internal,
    );
    let prepared = prepare_supersede_evidence_reference(
        &intent,
        PreparedIntentId::parse("prepared-1").unwrap(),
        &snapshot,
        UtcTimestamp::from_unix_millis(5_000),
    )
    .unwrap();

    let approval = EvidenceSupersessionApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        IdempotencyId::parse("synthetic-supersede-execute-1").unwrap(),
        true,
    )
    .unwrap();

    let result = execute_supersede_evidence_reference(
        &approval,
        &prepared,
        &snapshot,
        true,
        UtcTimestamp::from_unix_millis(6_000),
    );
    assert_eq!(
        result.unwrap_err(),
        EvidenceSupersessionError::SourceAlreadySuperseded
    );
}

#[test]
fn payload_digest_from_persisted_accepts_a_realistic_hex_digest_with_digits() {
    // A real SHA-256 hex digest is a mix of digits and a-f letters, not just
    // letters -- confirms `from_persisted`'s validation doesn't accidentally
    // reject the digit half of the hex alphabet.
    let digest = pmc_domain::evidence::EvidenceSupersessionPayloadDigest::from_persisted(
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned(),
    );
    assert!(digest.is_ok());
}

#[test]
fn approval_rejects_a_non_head_of_products_actor() {
    let result = EvidenceSupersessionApproval::new(
        PreparedIntentId::parse("prepared-1").unwrap(),
        AuditActor::PolicyAuthorizedSystem,
        pmc_domain::evidence::EvidenceSupersessionPayloadDigest::from_persisted("a".repeat(64))
            .unwrap(),
        IdempotencyId::parse("synthetic-supersede-execute-1").unwrap(),
        true,
    );
    assert_eq!(
        result.unwrap_err(),
        EvidenceSupersessionError::UnauthorizedActor
    );
}

#[test]
fn approval_rejects_missing_confirmation() {
    let result = EvidenceSupersessionApproval::new(
        PreparedIntentId::parse("prepared-1").unwrap(),
        AuditActor::HeadOfProducts,
        pmc_domain::evidence::EvidenceSupersessionPayloadDigest::from_persisted("a".repeat(64))
            .unwrap(),
        IdempotencyId::parse("synthetic-supersede-execute-1").unwrap(),
        false,
    );
    assert_eq!(
        result.unwrap_err(),
        EvidenceSupersessionError::MissingConfirmation
    );
}
