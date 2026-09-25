//! v45: a support witness binds each Evidence reference's aggregate version
//! (DG3: the exact preview and hash bind source revisions), so a change to
//! the reference that leaves its verification tuple identical still changes
//! the payload digest an approval acknowledges.

use pmc_domain::{
    classification::DataClassification,
    identity::{ActionId, AggregateVersion, EvidenceReferenceId, PreparedIntentId},
    time::UtcTimestamp,
    work_management::{
        EvidenceOrJudgment, EvidenceReferenceMetadata, EvidenceRole, EvidenceVerification,
        IntegrityDigest, WorkManagementOperation, WorkManagementPreparedIntent,
    },
};

fn metadata(version: u64) -> EvidenceReferenceMetadata {
    EvidenceReferenceMetadata::new(
        EvidenceReferenceId::parse("evidence-1").unwrap(),
        AggregateVersion::new(version).unwrap(),
        DataClassification::Internal,
        EvidenceRole::ActionCompletion,
        EvidenceVerification::Verified {
            verified_at: UtcTimestamp::from_unix_millis(90),
            integrity_digest: IntegrityDigest::parse("a".repeat(64)).unwrap(),
        },
    )
}

fn prepared(version: u64) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("prepared-1").unwrap(),
        WorkManagementOperation::CompleteAction {
            action_id: ActionId::parse("action-1").unwrap(),
            action_version: AggregateVersion::initial(),
        },
        DataClassification::Internal,
        Some(
            EvidenceOrJudgment::new(vec![metadata(version)], Vec::new())
                .unwrap()
                .evaluate_evidence_required()
                .unwrap(),
        ),
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap()
}

#[test]
fn the_payload_digest_binds_the_evidence_source_version() {
    assert_eq!(
        metadata(3).source_version(),
        AggregateVersion::new(3).unwrap()
    );
    assert_eq!(prepared(3).payload_digest(), prepared(3).payload_digest());
    assert_ne!(
        prepared(3).payload_digest(),
        prepared(4).payload_digest(),
        "two witnesses that differ only in source revision must not share a digest"
    );
}
