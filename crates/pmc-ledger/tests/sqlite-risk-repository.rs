use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_domain::{
    audit::AuditActor,
    classification::DataClassification,
    identity::{
        AggregateVersion, ApprovalReceiptId, AuditEventId, CorrelationId, IdempotencyId, IssueId,
        PreparedIntentId, RiskId,
    },
    issues::{
        ApproveAndExecuteLowerIssueClassification, CreateIssue, IssueDetails,
        IssueEvidenceAuthorityError, IssueEvidenceAuthorityPort, IssueExecutionPolicy,
        IssueExecutionPolicyPort, IssueOperationContext, IssueTitle,
        PrepareLowerIssueClassification, RecordedIssueClassification,
    },
    risks::{
        AllowRiskEvidence, ApproveAndExecuteCloseRisk, ApproveAndExecuteRecordRiskOccurrence,
        CreateRisk, PrepareCloseRisk, PrepareRecordRiskOccurrence, RecordedRiskClassification,
        RejectRiskPreparedIntent, RiskDetails, RiskExecutionPolicy, RiskExecutionPolicyPort,
        RiskOperationContext, RiskTitle,
    },
    time::UtcTimestamp,
    work_management::{
        ApprovalAuthorizationPort, ApprovalConfirmation, RiskState, WorkManagementApproval,
        WorkManagementOperation, WorkManagementPreparedIntent, WorkManagementRationale,
    },
};
use pmc_ledger::sqlite::SqliteProductLedger;
use rusqlite::Connection;

#[derive(Clone, Copy)]
struct AllowApproval;
impl ApprovalAuthorizationPort for AllowApproval {
    fn authorize(&self, actor: AuditActor) -> bool {
        actor == AuditActor::HeadOfProducts
    }
}

#[derive(Clone, Copy)]
struct FixedRiskPolicy(RiskExecutionPolicy);
impl RiskExecutionPolicyPort for FixedRiskPolicy {
    fn current_policy(&self, _: &WorkManagementOperation) -> RiskExecutionPolicy {
        self.0
    }
}

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic test clock must follow the Unix epoch")
            .as_nanos();
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-risk-repository-{nonce}-{sequence}.sqlite3"
        )))
    }
}

fn create_risk() -> CreateRisk {
    CreateRisk {
        id: RiskId::parse("synthetic-risk-1").unwrap(),
        title: RiskTitle::parse("Synthetic delivery risk").unwrap(),
        details: RiskDetails::parse("Synthetic only; no organizational data.").unwrap(),
        classification: DataClassification::Internal,
        context: RiskOperationContext {
            idempotency_id: IdempotencyId::parse("synthetic-risk-create-1").unwrap(),
            correlation_id: CorrelationId::parse("synthetic-risk-correlation-1").unwrap(),
        },
    }
}
#[test]
fn writer_creates_risk_and_reopens_losslessly() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    let outcome = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    assert_eq!(outcome.record.id(), &command.id);
    assert_eq!(outcome.record.version().get(), 1);
    assert_eq!(outcome.audit_events.len(), 1);
    let retry = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_risk(
            command.clone(),
            AuditEventId::parse("ignored-risk-audit").unwrap(),
            UtcTimestamp::from_unix_millis(999),
        )
        .unwrap();
    assert_eq!(retry, outcome);
    let mut altered = command.clone();
    altered.details = RiskDetails::parse("Altered synthetic details.").unwrap();
    assert!(SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_risk(
            altered,
            AuditEventId::parse("altered-risk-audit").unwrap(),
            UtcTimestamp::from_unix_millis(200),
        )
        .is_err());
    let snapshot = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_risk_persistence_snapshot()
        .unwrap();
    assert_eq!(snapshot.risks(), &[outcome.record]);
}

#[test]
fn reader_ignores_unrelated_standalone_issues() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-standalone").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('synthetic-standalone-issue','issue',1,'internal',100,100)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO issues (id,source_risk_id,recurrence_of_id,title,details,state,resolution_type,resolution_rationale) VALUES ('synthetic-standalone-issue',NULL,NULL,'Synthetic standalone issue','Synthetic only; no organizational data.','open',NULL,NULL)",
            [],
        )
        .unwrap();
    drop(connection);

    let snapshot = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_risk_persistence_snapshot()
        .unwrap();
    assert_eq!(snapshot.risks().len(), 1);
    assert!(snapshot.occurrence_issues().is_empty());
    assert!(snapshot.links().is_empty());
}

#[test]
fn reader_rejects_an_occurred_risk_until_its_h2a_bundle_is_durably_typed() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-1").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "UPDATE aggregate_registry SET version=2 WHERE id=?1",
            [command.id.as_str()],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE risks SET state='occurred' WHERE id=?1",
            [command.id.as_str()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO aggregate_registry (id,aggregate_type,version,classification,created_at,updated_at) VALUES ('synthetic-occurred-issue','issue',1,'internal',100,100)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO issues (id,source_risk_id,recurrence_of_id,title,details,state,resolution_type,resolution_rationale) VALUES ('synthetic-occurred-issue',?1,NULL,'Synthetic delivery risk','Synthetic only; no organizational data.','open',NULL,NULL)",
            [command.id.as_str()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO risk_issue_links (risk_id,issue_id,risk_version,issue_version,classification) VALUES (?1,'synthetic-occurred-issue',2,1,'internal')",
            [command.id.as_str()],
        )
        .unwrap();
    drop(connection);
    assert!(SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_risk_persistence_snapshot()
        .is_err());
}

#[test]
fn reader_rejects_any_unbound_risk_h2a_replay_authority() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-unbound").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let connection = Connection::open(&ledger.0).unwrap();
    connection
        .execute(
            "INSERT INTO risk_h2a_replay_operations (idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,terminal_cause,error_code,error_message_key,error_correlation_id,error_retryable) VALUES ('synthetic-unbound-risk-h2a','prepare_occurrence','synthetic-unbound-correlation',0,'terminal',NULL,'policy_denied', 'SECURITY_POLICY_DENIED','risk.policy_denied','synthetic-unbound-correlation',0)",
            [],
        )
        .unwrap();
    drop(connection);

    assert!(SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_risk_persistence_snapshot()
        .is_err());
}

#[test]
fn ordinary_risk_loader_rejects_v11_terminal_denial_until_the_typed_loader_exists() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-v11").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let connection = Connection::open(&ledger.0).unwrap();
    connection.execute_batch(&format!(
        "INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,expires_at,created_at) VALUES('synthetic-v11-prepared',1,'record_risk_occurrence','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','internal','allowed','not_cancellable_after_submit','head_of_products',100,0);\
         INSERT INTO prepared_intent_targets(prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES('synthetic-v11-prepared',0,'risk','{}',1);\
         INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES('synthetic-v11-audit',1,'head_of_products','work_management','risk.occurrence_denied','risk','{}','synthetic-v11-correlation','denied','not_required','not_attempted','none');\
         INSERT INTO risk_h2a_v11_terminal_denials(execute_idempotency_id,prepared_intent_id,risk_id,risk_version,operation,acknowledged_digest,correlation_id,policy_denial_audit_id,error_code,error_message_key,error_retryable) VALUES('synthetic-v11-execute','synthetic-v11-prepared','{}',1,'record_occurrence','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','synthetic-v11-correlation','synthetic-v11-audit','SECURITY_POLICY_DENIED','risk.policy_denied',0);",
        command.id.as_str(), command.id.as_str(), command.id.as_str()
    )).unwrap();
    drop(connection);

    assert!(SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_risk_persistence_snapshot()
        .is_err());
}

#[test]
fn typed_v11_loader_rehydrates_a_canonical_record_occurrence_denial() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-v11-loader").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-v11-loader-prepared").unwrap(),
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            issue_id: IssueId::parse("synthetic-v11-loader-issue").unwrap(),
            issue_classification: DataClassification::Internal,
        },
        DataClassification::Internal,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let connection = Connection::open(&ledger.0).unwrap();
    connection.execute_batch(&format!(
        "INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,expires_at,created_at) VALUES('{}',1,'record_risk_occurrence','{}','internal','allowed','not_cancellable_after_submit','head_of_products',{},0);\
         INSERT INTO prepared_intent_targets(prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES('{}',0,'risk','{}',1);\
         INSERT INTO prepared_work_management_payloads(prepared_intent_id,primary_id,primary_version,created_id,created_classification) VALUES('{}','{}',1,'synthetic-v11-loader-issue','internal');\
         INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES('synthetic-v11-loader-audit',1,'head_of_products','work_management','risk.occurrence_denied','risk','{}','synthetic-v11-loader-correlation','denied','not_required','not_attempted','none');\
         INSERT INTO risk_h2a_v11_terminal_denials(execute_idempotency_id,prepared_intent_id,risk_id,risk_version,operation,acknowledged_digest,correlation_id,policy_denial_audit_id,error_code,error_message_key,error_retryable) VALUES('synthetic-v11-loader-execute','{}','{}',1,'record_occurrence','{}','synthetic-v11-loader-correlation','synthetic-v11-loader-audit','SECURITY_POLICY_DENIED','risk.policy_denied',0);",
        prepared.id().as_str(), prepared.payload_digest().as_str(), prepared.preview().expires_at().unix_millis(), prepared.id().as_str(), command.id.as_str(), prepared.id().as_str(), command.id.as_str(), command.id.as_str(), prepared.id().as_str(), command.id.as_str(), prepared.payload_digest().as_str()
    )).unwrap();
    drop(connection);
    assert!(SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_risk_h2a_runtime_snapshot()
        .is_ok());
}

#[test]
fn typed_v11_loader_rehydrates_a_canonical_close_denial() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-v11-close-loader").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-v11-close-loader-prepared").unwrap(),
        WorkManagementOperation::CloseRisk {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            rationale: pmc_domain::work_management::WorkManagementRationale::parse(
                "Synthetic close rationale",
            )
            .unwrap(),
        },
        DataClassification::Internal,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let connection = Connection::open(&ledger.0).unwrap();
    connection.execute_batch(&format!(
        "INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,expires_at,created_at) VALUES('{}',1,'close_risk','{}','internal','allowed','not_cancellable_after_submit','head_of_products',{},0);\
         INSERT INTO prepared_intent_targets(prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES('{}',0,'risk','{}',1);\
         INSERT INTO prepared_work_management_payloads(prepared_intent_id,primary_id,primary_version,rationale) VALUES('{}','{}',1,'Synthetic close rationale');\
         INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES('synthetic-v11-close-loader-audit',1,'head_of_products','work_management','risk.close_denied','risk','{}','synthetic-v11-close-loader-correlation','denied','not_required','not_attempted','none');\
         INSERT INTO risk_h2a_v11_terminal_denials(execute_idempotency_id,prepared_intent_id,risk_id,risk_version,operation,acknowledged_digest,correlation_id,policy_denial_audit_id,error_code,error_message_key,error_retryable) VALUES('synthetic-v11-close-loader-execute','{}','{}',1,'close','{}','synthetic-v11-close-loader-correlation','synthetic-v11-close-loader-audit','SECURITY_POLICY_DENIED','risk.policy_denied',0);",
        prepared.id().as_str(), prepared.payload_digest().as_str(), prepared.preview().expires_at().unix_millis(), prepared.id().as_str(), command.id.as_str(), prepared.id().as_str(), command.id.as_str(), command.id.as_str(), prepared.id().as_str(), command.id.as_str(), prepared.payload_digest().as_str()
    )).unwrap();
    drop(connection);
    assert!(SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_risk_h2a_runtime_snapshot()
        .is_ok());
}

#[test]
fn prepare_record_risk_occurrence_persists_and_replays_the_exact_preview() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-prepare-occurrence").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let context = RiskOperationContext {
        idempotency_id: IdempotencyId::parse("synthetic-prepare-occurrence-idempotency").unwrap(),
        correlation_id: CorrelationId::parse("synthetic-prepare-occurrence-correlation").unwrap(),
    };
    let issue_id = IssueId::parse("synthetic-prepare-occurrence-issue").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-prepare-occurrence-prepared").unwrap(),
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            issue_id: issue_id.clone(),
            issue_classification: command.classification,
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let prepare_command = PrepareRecordRiskOccurrence {
        risk_id: command.id.clone(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        issue_id,
        context,
    };
    let first = ledger_handle
        .prepare_record_risk_occurrence(prepare_command.clone(), prepared.clone())
        .unwrap();
    assert_eq!(first, prepared);
    let replay = ledger_handle
        .prepare_record_risk_occurrence(prepare_command, prepared.clone())
        .unwrap();
    assert_eq!(replay, prepared);
}

#[test]
fn prepare_record_risk_occurrence_rejects_a_differing_replay() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-prepare-occurrence-conflict").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let context = RiskOperationContext {
        idempotency_id: IdempotencyId::parse("synthetic-prepare-occurrence-conflict-idempotency")
            .unwrap(),
        correlation_id: CorrelationId::parse("synthetic-prepare-occurrence-conflict-correlation")
            .unwrap(),
    };
    let issue_id = IssueId::parse("synthetic-prepare-occurrence-conflict-issue").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-prepare-occurrence-conflict-prepared").unwrap(),
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            issue_id: issue_id.clone(),
            issue_classification: command.classification,
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let prepare_command = PrepareRecordRiskOccurrence {
        risk_id: command.id.clone(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        issue_id,
        context: context.clone(),
    };
    ledger_handle
        .prepare_record_risk_occurrence(prepare_command, prepared)
        .unwrap();

    let altered_issue_id =
        IssueId::parse("synthetic-prepare-occurrence-conflict-issue-altered").unwrap();
    let altered_prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-prepare-occurrence-conflict-prepared-altered").unwrap(),
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            issue_id: altered_issue_id.clone(),
            issue_classification: command.classification,
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let altered_command = PrepareRecordRiskOccurrence {
        risk_id: command.id.clone(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        issue_id: altered_issue_id,
        context,
    };
    assert!(ledger_handle
        .prepare_record_risk_occurrence(altered_command, altered_prepared)
        .is_err());
}

/// A second call with the exact same idempotency ID, command scalars, and
/// even the same `prepared.id()` must still be rejected as a conflict if the
/// supplied preview's actual payload differs from what was durably
/// persisted -- `PrepareRecordRiskOccurrence` itself carries no
/// `issue_classification` field (it is derived, not part of the command), so
/// a caller could otherwise smuggle a different classification through an
/// "exact replay" that only compared command scalars and the prepared ID.
#[test]
fn prepare_record_risk_occurrence_rejects_a_same_id_replay_with_a_different_payload_digest() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-prepare-occurrence-digest").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let context = RiskOperationContext {
        idempotency_id: IdempotencyId::parse("synthetic-prepare-occurrence-digest-idempotency")
            .unwrap(),
        correlation_id: CorrelationId::parse("synthetic-prepare-occurrence-digest-correlation")
            .unwrap(),
    };
    let issue_id = IssueId::parse("synthetic-prepare-occurrence-digest-issue").unwrap();
    let prepared_id =
        PreparedIntentId::parse("synthetic-prepare-occurrence-digest-prepared").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        prepared_id.clone(),
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            issue_id: issue_id.clone(),
            issue_classification: DataClassification::Internal,
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    ledger_handle
        .prepare_record_risk_occurrence(
            PrepareRecordRiskOccurrence {
                risk_id: command.id.clone(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                issue_id: issue_id.clone(),
                context: context.clone(),
            },
            prepared,
        )
        .unwrap();

    // Same prepared ID, same command scalars -- but a different embedded
    // issue_classification, which the command itself never carries and the
    // old scalar-only replay check never compared.
    let differing_digest_prepared = WorkManagementPreparedIntent::prepare(
        prepared_id,
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            issue_id: issue_id.clone(),
            issue_classification: DataClassification::Confidential,
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    assert!(ledger_handle
        .prepare_record_risk_occurrence(
            PrepareRecordRiskOccurrence {
                risk_id: command.id.clone(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                issue_id,
                context,
            },
            differing_digest_prepared,
        )
        .is_err());
}

#[test]
fn prepare_record_risk_occurrence_rejects_a_topology_mismatched_preview() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-prepare-occurrence-topology").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let context = RiskOperationContext {
        idempotency_id: IdempotencyId::parse("synthetic-prepare-occurrence-topology-idempotency")
            .unwrap(),
        correlation_id: CorrelationId::parse("synthetic-prepare-occurrence-topology-correlation")
            .unwrap(),
    };
    let mismatched_prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-prepare-occurrence-topology-prepared").unwrap(),
        WorkManagementOperation::CloseRisk {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            rationale: WorkManagementRationale::parse("Synthetic mismatched rationale").unwrap(),
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let prepare_command = PrepareRecordRiskOccurrence {
        risk_id: command.id.clone(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        issue_id: IssueId::parse("synthetic-prepare-occurrence-topology-issue").unwrap(),
        context,
    };
    assert!(ledger_handle
        .prepare_record_risk_occurrence(prepare_command, mismatched_prepared)
        .is_err());
}

#[test]
fn prepare_close_risk_persists_and_replays_the_exact_preview() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-prepare-close").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let context = RiskOperationContext {
        idempotency_id: IdempotencyId::parse("synthetic-prepare-close-idempotency").unwrap(),
        correlation_id: CorrelationId::parse("synthetic-prepare-close-correlation").unwrap(),
    };
    let rationale = WorkManagementRationale::parse("Synthetic close rationale").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-prepare-close-prepared").unwrap(),
        WorkManagementOperation::CloseRisk {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            rationale: rationale.clone(),
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let prepare_command = PrepareCloseRisk {
        risk_id: command.id.clone(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        rationale,
        context,
    };
    let first = ledger_handle
        .prepare_close_risk(prepare_command.clone(), prepared.clone())
        .unwrap();
    assert_eq!(first, prepared);
    let replay = ledger_handle
        .prepare_close_risk(prepare_command, prepared.clone())
        .unwrap();
    assert_eq!(replay, prepared);
}

#[test]
fn restart_recovers_a_v13_prepared_occurrence_preview() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    let context = RiskOperationContext {
        idempotency_id: IdempotencyId::parse("synthetic-restart-occurrence-idempotency").unwrap(),
        correlation_id: CorrelationId::parse("synthetic-restart-occurrence-correlation").unwrap(),
    };
    let issue_id = IssueId::parse("synthetic-restart-occurrence-issue").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-restart-occurrence-prepared").unwrap(),
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            issue_id: issue_id.clone(),
            issue_classification: command.classification,
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let prepare_command = PrepareRecordRiskOccurrence {
        risk_id: command.id.clone(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        issue_id,
        context,
    };
    {
        let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
        ledger_handle
            .create_risk(
                command.clone(),
                AuditEventId::parse("synthetic-risk-audit-restart-occurrence").unwrap(),
                UtcTimestamp::from_unix_millis(100),
            )
            .unwrap();
        ledger_handle
            .prepare_record_risk_occurrence(prepare_command, prepared.clone())
            .unwrap();
    }
    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_risk_h2a_runtime_snapshot().unwrap();
    assert_eq!(snapshot.prepared(), &[prepared]);
}

#[test]
fn restart_recovers_a_v13_prepared_close_preview() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    let context = RiskOperationContext {
        idempotency_id: IdempotencyId::parse("synthetic-restart-close-idempotency").unwrap(),
        correlation_id: CorrelationId::parse("synthetic-restart-close-correlation").unwrap(),
    };
    let rationale = WorkManagementRationale::parse("Synthetic restart close rationale").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-restart-close-prepared").unwrap(),
        WorkManagementOperation::CloseRisk {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            rationale: rationale.clone(),
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    let prepare_command = PrepareCloseRisk {
        risk_id: command.id.clone(),
        expected_version: pmc_domain::identity::AggregateVersion::initial(),
        rationale,
        context,
    };
    {
        let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
        ledger_handle
            .create_risk(
                command.clone(),
                AuditEventId::parse("synthetic-risk-audit-restart-close").unwrap(),
                UtcTimestamp::from_unix_millis(100),
            )
            .unwrap();
        ledger_handle
            .prepare_close_risk(prepare_command, prepared.clone())
            .unwrap();
    }
    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    let snapshot = reopened.load_risk_h2a_runtime_snapshot().unwrap();
    assert_eq!(snapshot.prepared(), &[prepared]);
}

fn approval_for(
    prepared: &WorkManagementPreparedIntent,
    idempotency_id: &IdempotencyId,
) -> WorkManagementApproval {
    WorkManagementApproval::new(
        prepared.id().clone(),
        AuditActor::HeadOfProducts,
        prepared.payload_digest().clone(),
        idempotency_id.clone(),
        Some(ApprovalConfirmation::Confirmed),
    )
    .unwrap()
}

#[test]
fn approve_and_execute_record_risk_occurrence_persists_and_replays() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-execute-occurrence").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let issue_id = IssueId::parse("synthetic-execute-occurrence-issue").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-execute-occurrence-prepared").unwrap(),
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            issue_id: issue_id.clone(),
            issue_classification: command.classification,
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    ledger_handle
        .prepare_record_risk_occurrence(
            PrepareRecordRiskOccurrence {
                risk_id: command.id.clone(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                issue_id,
                context: RiskOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-execute-occurrence-prepare-idempotency",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-execute-occurrence-prepare-correlation",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-execute-occurrence-execute-idempotency").unwrap();
    let command_execute = ApproveAndExecuteRecordRiskOccurrence {
        approval: approval_for(&prepared, &execute_idempotency_id),
        context: RiskOperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse(
                "synthetic-execute-occurrence-execute-correlation",
            )
            .unwrap(),
        },
    };
    let audit_ids = [
        AuditEventId::parse("synthetic-execute-occurrence-audit-0").unwrap(),
        AuditEventId::parse("synthetic-execute-occurrence-audit-1").unwrap(),
        AuditEventId::parse("synthetic-execute-occurrence-audit-2").unwrap(),
    ];
    let receipt_id = ApprovalReceiptId::parse("synthetic-execute-occurrence-receipt").unwrap();
    let outcome = ledger_handle
        .approve_and_execute_record_risk_occurrence(
            command_execute.clone(),
            audit_ids.clone(),
            receipt_id.clone(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            FixedRiskPolicy(RiskExecutionPolicy::Allowed),
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap();
    assert_eq!(outcome.risk.id(), &command.id);
    assert_eq!(outcome.issue.source_risk_id(), Some(&command.id));
    assert_eq!(outcome.approval_receipt_id, receipt_id);

    let replay = ledger_handle
        .approve_and_execute_record_risk_occurrence(
            command_execute,
            audit_ids,
            receipt_id,
            UtcTimestamp::from_unix_millis(999),
            AllowApproval,
            FixedRiskPolicy(RiskExecutionPolicy::Allowed),
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap();
    assert_eq!(replay, outcome);

    let connection = Connection::open(&ledger.0).unwrap();
    let state: String = connection
        .query_row(
            "SELECT state FROM risks WHERE id=?1",
            [command.id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "occurred");
    let issue_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM issues WHERE source_risk_id=?1",
            [command.id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(issue_count, 1);
    let receipt_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM approval_receipts WHERE id='synthetic-execute-occurrence-receipt'", [], |row| row.get(0))
        .unwrap();
    assert_eq!(receipt_count, 1);
}

/// A replay must return the persisted audits' own correlation ID, not the
/// one the replaying caller happens to supply -- V14 execute replay keys on
/// prepared ID, actor, and acknowledged digest only, never correlation ID,
/// so a differing correlation ID here is a legitimate replay and the
/// returned `AuditEvent`s must reflect what actually happened.
#[test]
fn approve_and_execute_record_risk_occurrence_replay_returns_the_original_correlation_id() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-execute-occurrence-corr").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let issue_id = IssueId::parse("synthetic-execute-occurrence-corr-issue").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-execute-occurrence-corr-prepared").unwrap(),
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            issue_id: issue_id.clone(),
            issue_classification: command.classification,
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    ledger_handle
        .prepare_record_risk_occurrence(
            PrepareRecordRiskOccurrence {
                risk_id: command.id.clone(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                issue_id,
                context: RiskOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-execute-occurrence-corr-prepare-idempotency",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-execute-occurrence-corr-prepare-correlation",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-execute-occurrence-corr-execute-idempotency").unwrap();
    let original_correlation_id =
        CorrelationId::parse("synthetic-execute-occurrence-corr-original").unwrap();
    let original_context = RiskOperationContext {
        idempotency_id: execute_idempotency_id.clone(),
        correlation_id: original_correlation_id.clone(),
    };
    let audit_ids = [
        AuditEventId::parse("synthetic-execute-occurrence-corr-audit-0").unwrap(),
        AuditEventId::parse("synthetic-execute-occurrence-corr-audit-1").unwrap(),
        AuditEventId::parse("synthetic-execute-occurrence-corr-audit-2").unwrap(),
    ];
    let receipt_id = ApprovalReceiptId::parse("synthetic-execute-occurrence-corr-receipt").unwrap();
    let approval = approval_for(&prepared, &execute_idempotency_id);
    let outcome = ledger_handle
        .approve_and_execute_record_risk_occurrence(
            ApproveAndExecuteRecordRiskOccurrence {
                approval: approval.clone(),
                context: original_context.clone(),
            },
            audit_ids.clone(),
            receipt_id.clone(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            FixedRiskPolicy(RiskExecutionPolicy::Allowed),
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap();
    assert!(outcome
        .audit_events
        .iter()
        .all(|audit| audit.correlation_id() == &original_correlation_id));

    let replay = ledger_handle
        .approve_and_execute_record_risk_occurrence(
            ApproveAndExecuteRecordRiskOccurrence {
                approval,
                context: RiskOperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse(
                        "synthetic-execute-occurrence-corr-different",
                    )
                    .unwrap(),
                },
            },
            audit_ids,
            receipt_id,
            UtcTimestamp::from_unix_millis(999),
            AllowApproval,
            FixedRiskPolicy(RiskExecutionPolicy::Allowed),
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap();
    assert_eq!(replay, outcome);
    assert!(replay
        .audit_events
        .iter()
        .all(|audit| audit.correlation_id() == &original_correlation_id));
}

#[test]
fn approve_and_execute_record_risk_occurrence_denies_and_persists_v11_terminal() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-execute-occurrence-denied").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let issue_id = IssueId::parse("synthetic-execute-occurrence-denied-issue").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-execute-occurrence-denied-prepared").unwrap(),
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            issue_id: issue_id.clone(),
            issue_classification: command.classification,
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    ledger_handle
        .prepare_record_risk_occurrence(
            PrepareRecordRiskOccurrence {
                risk_id: command.id.clone(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                issue_id,
                context: RiskOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-execute-occurrence-denied-prepare-idempotency",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-execute-occurrence-denied-prepare-correlation",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-execute-occurrence-denied-execute-idempotency").unwrap();
    let command_execute = ApproveAndExecuteRecordRiskOccurrence {
        approval: approval_for(&prepared, &execute_idempotency_id),
        context: RiskOperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse(
                "synthetic-execute-occurrence-denied-execute-correlation",
            )
            .unwrap(),
        },
    };
    let audit_ids = [
        AuditEventId::parse("synthetic-execute-occurrence-denied-audit-0").unwrap(),
        AuditEventId::parse("synthetic-execute-occurrence-denied-audit-1").unwrap(),
        AuditEventId::parse("synthetic-execute-occurrence-denied-audit-2").unwrap(),
    ];
    let receipt_id =
        ApprovalReceiptId::parse("synthetic-execute-occurrence-denied-receipt").unwrap();
    let first = ledger_handle.approve_and_execute_record_risk_occurrence(
        command_execute.clone(),
        audit_ids.clone(),
        receipt_id.clone(),
        UtcTimestamp::from_unix_millis(200),
        AllowApproval,
        FixedRiskPolicy(RiskExecutionPolicy::Denied),
        AllowRiskEvidence,
        RecordedRiskClassification,
    );
    assert!(first.is_err());

    let connection = Connection::open(&ledger.0).unwrap();
    let denial_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM risk_h2a_v11_terminal_denials WHERE prepared_intent_id=?1 AND operation='record_occurrence'",
            [prepared.id().as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(denial_count, 1);
    drop(connection);

    let replay = ledger_handle.approve_and_execute_record_risk_occurrence(
        command_execute,
        audit_ids,
        receipt_id,
        UtcTimestamp::from_unix_millis(999),
        AllowApproval,
        FixedRiskPolicy(RiskExecutionPolicy::Denied),
        AllowRiskEvidence,
        RecordedRiskClassification,
    );
    assert!(replay.is_err());
    let connection = Connection::open(&ledger.0).unwrap();
    let denial_count_after_replay: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM risk_h2a_v11_terminal_denials WHERE prepared_intent_id=?1 AND operation='record_occurrence'",
            [prepared.id().as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(denial_count_after_replay, 1);
}

#[test]
fn approve_and_execute_close_risk_persists_and_replays() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-execute-close").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let rationale = WorkManagementRationale::parse("Synthetic execute close rationale").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-execute-close-prepared").unwrap(),
        WorkManagementOperation::CloseRisk {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            rationale: rationale.clone(),
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    ledger_handle
        .prepare_close_risk(
            PrepareCloseRisk {
                risk_id: command.id.clone(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                rationale,
                context: RiskOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-execute-close-prepare-idempotency",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-execute-close-prepare-correlation",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();

    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-execute-close-execute-idempotency").unwrap();
    let command_execute = ApproveAndExecuteCloseRisk {
        approval: approval_for(&prepared, &execute_idempotency_id),
        context: RiskOperationContext {
            idempotency_id: execute_idempotency_id,
            correlation_id: CorrelationId::parse("synthetic-execute-close-execute-correlation")
                .unwrap(),
        },
    };
    let audit_id = AuditEventId::parse("synthetic-execute-close-audit-0").unwrap();
    let receipt_id = ApprovalReceiptId::parse("synthetic-execute-close-receipt").unwrap();
    let outcome = ledger_handle
        .approve_and_execute_close_risk(
            command_execute.clone(),
            audit_id.clone(),
            receipt_id.clone(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            FixedRiskPolicy(RiskExecutionPolicy::Allowed),
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap();
    assert_eq!(outcome.record.id(), &command.id);
    assert_eq!(outcome.approval_receipt_id, Some(receipt_id.clone()));

    let replay = ledger_handle
        .approve_and_execute_close_risk(
            command_execute,
            audit_id,
            receipt_id,
            UtcTimestamp::from_unix_millis(999),
            AllowApproval,
            FixedRiskPolicy(RiskExecutionPolicy::Allowed),
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap();
    assert_eq!(replay, outcome);

    let connection = Connection::open(&ledger.0).unwrap();
    let state: String = connection
        .query_row(
            "SELECT state FROM risks WHERE id=?1",
            [command.id.as_str()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "closed");
}

/// Proves the positive counterpart of `reader_rejects_an_occurred_risk_until_its_h2a_bundle_is_durably_typed`:
/// once a Risk has genuinely occurred through the real execute writer (so a matching V14
/// execute-replay row backs it), a cold-opened `SqliteProductLedger` must rehydrate it losslessly
/// through the ordinary Risk snapshot loader, not merely refuse to infer it from an unbacked
/// `risks.state` value.
#[test]
fn reader_cold_rehydrates_a_genuinely_occurred_risk() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-cold-occurrence").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let issue_id = IssueId::parse("synthetic-cold-occurrence-issue").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-cold-occurrence-prepared").unwrap(),
        WorkManagementOperation::RecordRiskOccurrence {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            issue_id: issue_id.clone(),
            issue_classification: command.classification,
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    ledger_handle
        .prepare_record_risk_occurrence(
            PrepareRecordRiskOccurrence {
                risk_id: command.id.clone(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                issue_id,
                context: RiskOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-cold-occurrence-prepare-idempotency",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-cold-occurrence-prepare-correlation",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-cold-occurrence-execute-idempotency").unwrap();
    let outcome = ledger_handle
        .approve_and_execute_record_risk_occurrence(
            ApproveAndExecuteRecordRiskOccurrence {
                approval: approval_for(&prepared, &execute_idempotency_id),
                context: RiskOperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse(
                        "synthetic-cold-occurrence-execute-correlation",
                    )
                    .unwrap(),
                },
            },
            [
                AuditEventId::parse("synthetic-cold-occurrence-audit-0").unwrap(),
                AuditEventId::parse("synthetic-cold-occurrence-audit-1").unwrap(),
                AuditEventId::parse("synthetic-cold-occurrence-audit-2").unwrap(),
            ],
            ApprovalReceiptId::parse("synthetic-cold-occurrence-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            FixedRiskPolicy(RiskExecutionPolicy::Allowed),
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap();

    let snapshot = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_risk_persistence_snapshot()
        .unwrap();
    let link = pmc_domain::risks::RiskIssueLink::from_persisted(
        command.id.clone(),
        outcome.issue.id().clone(),
        outcome.risk.version(),
        outcome.issue.version(),
        outcome.risk.classification(),
    )
    .unwrap();
    let expected = pmc_domain::risks::RiskPersistenceSnapshot::try_new(
        vec![outcome.risk.clone()],
        vec![outcome.issue.clone()],
        vec![link],
    )
    .unwrap();
    assert_eq!(snapshot, expected);
}

/// Positive counterpart for the Closed state: a genuinely closed Risk (backed by a real V14
/// `execute_close` row) must cold-rehydrate, not just reject an unbacked `state='closed'` row.
#[test]
fn reader_cold_rehydrates_a_genuinely_closed_risk() {
    let ledger = SyntheticLedger::new();
    let command = create_risk();
    let mut ledger_handle = SqliteProductLedger::open(&ledger.0).unwrap();
    ledger_handle
        .create_risk(
            command.clone(),
            AuditEventId::parse("synthetic-risk-audit-cold-close").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    let rationale = WorkManagementRationale::parse("Synthetic cold-close rationale").unwrap();
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("synthetic-cold-close-prepared").unwrap(),
        WorkManagementOperation::CloseRisk {
            risk_id: command.id.clone(),
            risk_version: pmc_domain::identity::AggregateVersion::initial(),
            rationale: rationale.clone(),
        },
        command.classification,
        None,
        UtcTimestamp::from_unix_millis(0),
    )
    .unwrap();
    ledger_handle
        .prepare_close_risk(
            PrepareCloseRisk {
                risk_id: command.id.clone(),
                expected_version: pmc_domain::identity::AggregateVersion::initial(),
                rationale,
                context: RiskOperationContext {
                    idempotency_id: IdempotencyId::parse(
                        "synthetic-cold-close-prepare-idempotency",
                    )
                    .unwrap(),
                    correlation_id: CorrelationId::parse(
                        "synthetic-cold-close-prepare-correlation",
                    )
                    .unwrap(),
                },
            },
            prepared.clone(),
        )
        .unwrap();
    let execute_idempotency_id =
        IdempotencyId::parse("synthetic-cold-close-execute-idempotency").unwrap();
    let outcome = ledger_handle
        .approve_and_execute_close_risk(
            ApproveAndExecuteCloseRisk {
                approval: approval_for(&prepared, &execute_idempotency_id),
                context: RiskOperationContext {
                    idempotency_id: execute_idempotency_id,
                    correlation_id: CorrelationId::parse(
                        "synthetic-cold-close-execute-correlation",
                    )
                    .unwrap(),
                },
            },
            AuditEventId::parse("synthetic-cold-close-audit-0").unwrap(),
            ApprovalReceiptId::parse("synthetic-cold-close-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(200),
            AllowApproval,
            FixedRiskPolicy(RiskExecutionPolicy::Allowed),
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .unwrap();

    let snapshot = SqliteProductLedger::open(&ledger.0)
        .unwrap()
        .load_risk_persistence_snapshot()
        .unwrap();
    let expected = pmc_domain::risks::RiskPersistenceSnapshot::try_new(
        vec![outcome.record.clone()],
        vec![],
        vec![],
    )
    .unwrap();
    assert_eq!(snapshot, expected);
}

/// A standalone Issue leaves its pristine create state as soon as its
/// classification is lowered: the version moves to 2 while the state and both
/// resolution columns stay exactly as created. `IssueH1RuntimeSnapshot` --
/// which is only ever the H1 *create* shape -- then refuses the whole
/// snapshot. Risk close and Risk rejection create no Issue and must not
/// depend on it; before this test they rehydrated through it and became
/// permanently unavailable after one lowering.
#[test]
fn a_standalone_issue_that_left_its_create_state_does_not_take_close_or_rejection_with_it() {
    let ledger = SyntheticLedger::new();
    let risk = create_risk();
    let mut handle = SqliteProductLedger::open(&ledger.0).unwrap();
    handle
        .create_risk(
            risk.clone(),
            AuditEventId::parse("unbrick-risk-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    handle
        .create_issue(
            CreateIssue {
                id: IssueId::parse("unbrick-issue").unwrap(),
                title: IssueTitle::parse("Standalone issue that gets lowered").unwrap(),
                details: IssueDetails::parse("Synthetic only; no organizational data.").unwrap(),
                classification: DataClassification::Confidential,
                recurrence_of: None,
                context: issue_context("unbrick-issue-create"),
            },
            AuditEventId::parse("unbrick-issue-create-audit").unwrap(),
            UtcTimestamp::from_unix_millis(100),
        )
        .unwrap();
    assert!(
        handle.load_issue_h1_runtime_snapshot().is_ok(),
        "a freshly created standalone Issue is still at its create state"
    );

    lower_issue_classification(&mut handle);
    assert!(
        handle.load_issue_h1_runtime_snapshot().is_err(),
        "the lowered Issue is Open with no resolution, so it still matches the H1 query and is refused by its constructor -- the condition this test exists for"
    );

    // Rejection: prepare a close preview, then refuse it durably.
    let refused = close_preview("unbrick-refused-prepared");
    handle
        .prepare_close_risk(
            PrepareCloseRisk {
                risk_id: risk.id.clone(),
                expected_version: AggregateVersion::initial(),
                rationale: WorkManagementRationale::parse("Synthetic close rationale").unwrap(),
                context: risk_context("unbrick-prepare-refused"),
            },
            refused.clone(),
        )
        .unwrap();
    let rejection = handle
        .reject_risk_prepared_intent(
            RejectRiskPreparedIntent {
                prepared_id: refused.id().clone(),
                actor: AuditActor::HeadOfProducts,
                context: risk_context("unbrick-reject"),
            },
            AuditEventId::parse("unbrick-reject-audit").unwrap(),
            UtcTimestamp::from_unix_millis(2_000),
            AllowApproval,
        )
        .expect("a lowered standalone Issue must not make Risk rejection unavailable");
    assert_eq!(rejection.prepared_intent_id(), refused.id());

    // Close: prepare a second preview and execute it.
    let accepted = close_preview("unbrick-accepted-prepared");
    handle
        .prepare_close_risk(
            PrepareCloseRisk {
                risk_id: risk.id.clone(),
                expected_version: AggregateVersion::initial(),
                rationale: WorkManagementRationale::parse("Synthetic close rationale").unwrap(),
                context: risk_context("unbrick-prepare-accepted"),
            },
            accepted.clone(),
        )
        .unwrap();
    let execute_idempotency = IdempotencyId::parse("unbrick-close-execute").unwrap();
    let outcome = handle
        .approve_and_execute_close_risk(
            ApproveAndExecuteCloseRisk {
                approval: approval_for(&accepted, &execute_idempotency),
                context: RiskOperationContext {
                    idempotency_id: execute_idempotency,
                    correlation_id: CorrelationId::parse("unbrick-close-execute-correlation")
                        .unwrap(),
                },
            },
            AuditEventId::parse("unbrick-close-audit").unwrap(),
            ApprovalReceiptId::parse("unbrick-close-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(3_000),
            AllowApproval,
            FixedRiskPolicy(RiskExecutionPolicy::Allowed),
            AllowRiskEvidence,
            RecordedRiskClassification,
        )
        .expect("a lowered standalone Issue must not make Risk close unavailable");
    assert_eq!(outcome.record.state(), RiskState::Closed);

    // The lowered Issue is untouched by either Risk operation.
    let reopened = SqliteProductLedger::open(&ledger.0).unwrap();
    assert!(reopened.load_issue_h1_runtime_snapshot().is_err());
}

fn issue_context(idempotency: &str) -> IssueOperationContext {
    IssueOperationContext {
        idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
        correlation_id: CorrelationId::parse(format!("{idempotency}-correlation")).unwrap(),
    }
}

fn risk_context(idempotency: &str) -> RiskOperationContext {
    RiskOperationContext {
        idempotency_id: IdempotencyId::parse(idempotency).unwrap(),
        correlation_id: CorrelationId::parse(format!("{idempotency}-correlation")).unwrap(),
    }
}

fn close_preview(prepared_id: &str) -> WorkManagementPreparedIntent {
    WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse(prepared_id).unwrap(),
        WorkManagementOperation::CloseRisk {
            risk_id: RiskId::parse("synthetic-risk-1").unwrap(),
            risk_version: AggregateVersion::initial(),
            rationale: WorkManagementRationale::parse("Synthetic close rationale").unwrap(),
        },
        DataClassification::Internal,
        None,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap()
}

/// Lowers `unbrick-issue` from Confidential to Internal through its own H2a
/// loop, which bumps the aggregate version and leaves every lifecycle column
/// exactly as created.
fn lower_issue_classification(handle: &mut SqliteProductLedger) {
    let prepared = WorkManagementPreparedIntent::prepare(
        PreparedIntentId::parse("unbrick-lower-prepared").unwrap(),
        WorkManagementOperation::LowerIssueClassification {
            issue_id: IssueId::parse("unbrick-issue").unwrap(),
            issue_version: AggregateVersion::initial(),
            current_classification: DataClassification::Confidential,
            proposed_classification: DataClassification::Internal,
            rationale: WorkManagementRationale::parse("Synthetic lowering rationale").unwrap(),
        },
        DataClassification::Confidential,
        None,
        UtcTimestamp::from_unix_millis(1_000),
    )
    .unwrap();
    handle
        .prepare_lower_issue_classification(
            PrepareLowerIssueClassification {
                issue_id: IssueId::parse("unbrick-issue").unwrap(),
                expected_version: AggregateVersion::initial(),
                proposed_classification: DataClassification::Internal,
                rationale: WorkManagementRationale::parse("Synthetic lowering rationale").unwrap(),
                context: issue_context("unbrick-lower-prepare"),
            },
            prepared.clone(),
        )
        .unwrap();
    handle
        .approve_and_execute_lower_issue_classification(
            ApproveAndExecuteLowerIssueClassification {
                approval: WorkManagementApproval::new(
                    prepared.id().clone(),
                    AuditActor::HeadOfProducts,
                    prepared.payload_digest().clone(),
                    IdempotencyId::parse("unbrick-lower-execute").unwrap(),
                    Some(ApprovalConfirmation::Confirmed),
                )
                .unwrap(),
                context: issue_context("unbrick-lower-execute"),
            },
            AuditEventId::parse("unbrick-lower-audit").unwrap(),
            ApprovalReceiptId::parse("unbrick-lower-receipt").unwrap(),
            UtcTimestamp::from_unix_millis(1_100),
            AllowApproval,
            AllowIssue,
            AllowIssue,
            RecordedIssueClassification,
        )
        .unwrap();
}

#[derive(Clone, Copy)]
struct AllowIssue;
impl IssueExecutionPolicyPort for AllowIssue {
    fn current_policy(&self, _: &WorkManagementOperation) -> IssueExecutionPolicy {
        IssueExecutionPolicy::Allowed
    }
}
impl IssueEvidenceAuthorityPort for AllowIssue {
    fn resolve(
        &self,
        _: &pmc_domain::identity::EvidenceReferenceId,
    ) -> Result<pmc_domain::work_management::EvidenceReferenceMetadata, IssueEvidenceAuthorityError>
    {
        Err(IssueEvidenceAuthorityError::NotFound)
    }
}
