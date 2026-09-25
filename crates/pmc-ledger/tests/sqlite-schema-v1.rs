use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use pmc_ledger::sqlite::{
    LedgerOpenError, LedgerTransactionError, SqliteProductLedger, CURRENT_SCHEMA_VERSION,
};
use rusqlite::Connection;

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

struct SyntheticLedger(PathBuf);

impl SyntheticLedger {
    fn new(label: &str) -> Self {
        let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("synthetic clock must follow Unix epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "pmc-synthetic-schema-v1-{label}-{nonce}-{sequence}.sqlite3"
        )))
    }
}

impl Drop for SyntheticLedger {
    fn drop(&mut self) {
        for path in [
            self.0.clone(),
            PathBuf::from(format!("{}-wal", self.0.display())),
            PathBuf::from(format!("{}-shm", self.0.display())),
        ] {
            let _ = fs::remove_file(path);
        }
    }
}

#[test]
fn empty_ledger_bootstraps_the_exact_current_migration_and_authority_inventory() {
    let ledger = SyntheticLedger::new("inventory");
    drop(SqliteProductLedger::open(&ledger.0).expect("synthetic ledger must initialize"));
    let connection = Connection::open(&ledger.0).expect("synthetic ledger must be inspectable");

    let migrations = connection
        .prepare("SELECT version, migration_key, checksum FROM schema_migrations ORDER BY version")
        .expect("migration registry query must prepare")
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("migration registry must query")
        .collect::<Result<Vec<(i64, String, String)>, _>>()
        .expect("migration registry must decode");
    assert_eq!(
        migrations,
        vec![
            (
                1,
                "0001_initial_authoritative_ledger".to_owned(),
                "sha256:fb08ddf57769e73729ac5fcbe2bf7c260a69cea9857109896c1d42eb43e63614"
                    .to_owned()
            ),
            (
                2,
                "0002_action_replay_authority".to_owned(),
                "sha256:f3f21983eef98406ceba2f15cbc5e38baa8dae5ddc86a743be00ce715a1fd6f2"
                    .to_owned()
            ),
            (
                3,
                "0003_decision_h1_replay_authority".to_owned(),
                "sha256:1729f5d28e0f164a9dbd081b39807c00b06d99e5af3e22c18eb04ea7e9f0b1d1"
                    .to_owned()
            ),
            (
                4,
                "0004_decision_h2a_resolve_replay_authority".to_owned(),
                "sha256:29df3c1585c6cf6eb5c67b3671fe1c6cd5c9edc052d76b7f88a00890f89f2a90"
                    .to_owned()
            ),
            (
                5,
                "0005_decision_replay_global_ordinal_repair".to_owned(),
                "sha256:5d35fede24563856fd7b032f40ec30dc4cb063b7657c615886cfc040adb6242a"
                    .to_owned()
            ),
            (
                6,
                "0006_decision_h2a_evidence_snapshot".to_owned(),
                "sha256:9d18674f3320655d93d8044e18ad8647e233b55678190d9ff7ee3a552b8bbaf9"
                    .to_owned()
            ),
            (
                7,
                "0007_risk_h1_replay_authority".to_owned(),
                "sha256:bfb9e344dc4b163f85d369037595e903ade876173847ea4adf607376a61376db"
                    .to_owned()
            ),
            (
                8,
                "0008_decision_h2a_correlation_anchor".to_owned(),
                "sha256:8c108d437aa10f7849e19258ce32097c33e9fec3df520e34929ff0d742e58c3d"
                    .to_owned()
            ),
            (
                9,
                "0009_risk_h2a_replay_authority".to_owned(),
                "sha256:f8e5e3ac151e1a964d010efbd37ae9660c5de743966fe8213d6acbd42a41be8c"
                    .to_owned()
            ),
            (
                10,
                "0010_issue_h1_replay_authority".to_owned(),
                "sha256:b3a5f33cc27db4c7f1ac2443629cd6f4e8e175c92fec6743304c8a248951b41f"
                    .to_owned()
            ),
            (
                11,
                "0011_risk_h2a_terminal_denial_replacement".to_owned(),
                "sha256:13a07a5d79b04ec1351390c92ea038f2bb972ca9fffc8ffe86f8a4fdc8a4e37b"
                    .to_owned()
            ),
            (
                12,
                "0012_risk_h2a_terminal_denial_audit_binding".to_owned(),
                "sha256:5c9bba3be6c3b5f1d8c32983acd742f7525bfccf93f35fd5fac5a705a722b9ec"
                    .to_owned()
            ),
            (
                13,
                "0013_risk_h2a_prepare_replay_authority".to_owned(),
                "sha256:95b63650665ae9af449f0e4cf137a71be5b93a5434e92326df86f30e6c3d5feb"
                    .to_owned()
            ),
            (
                14,
                "0014_risk_h2a_execute_replay_authority".to_owned(),
                "sha256:9066baa82f9e5654b211fe97440fc0da7e85008aae389fcd488bcca3653a0647"
                    .to_owned()
            ),
            (
                15,
                "0015_issue_h2a_prepare_replay_authority".to_owned(),
                "sha256:5883e7987f005814e8665412731d2d4319e072ad2ce76ff427371f3f00585933"
                    .to_owned()
            ),
            (
                16,
                "0016_issue_h2a_execute_replay_authority".to_owned(),
                "sha256:c3e01c97aea0baa7db3a97fe5793685c9f88e87cdbd247c6ae70f27466d67690"
                    .to_owned()
            ),
            (
                17,
                "0017_portfolio_kpi_observation_inherited_classification_repair".to_owned(),
                "sha256:347c2841268624d37bf829036870ebdf90466967e99e14bdb5fcf280d9e5c933"
                    .to_owned()
            ),
            (
                18,
                "0018_evidence_reference_generalize".to_owned(),
                "sha256:846a9d1ab843313e18fd22cfa02bd6ae50f02eb417448f3ae4b81a2a110a2885"
                    .to_owned()
            ),
            (
                19,
                "0019_evidence_link_replay_authority".to_owned(),
                "sha256:99b7c3d4db5708cd1747b5d76db1f1625d4dd8387d14efcd6bdf2ddaab3367d7"
                    .to_owned()
            ),
            (
                20,
                "0020_evidence_verification_update_replay_authority".to_owned(),
                "sha256:c918a6ee8293cfe8e859d228d2db23ab622602444a57c95c7d68bfc1a0bdfe5e"
                    .to_owned()
            ),
            (
                21,
                "0021_portfolio_h2a_lower_classification_replay_authority".to_owned(),
                "sha256:a28c4622f509b5ef4354bfb5b7bd9b8d898c84357e5f077e138a7478bc75954e"
                    .to_owned()
            ),
            (
                22,
                "0022_portfolio_siblings_h2a_lower_classification_replay_authority".to_owned(),
                "sha256:47cca153a2113e05b3f376906f5c0bacfdfe7a046ddd940ea6a517fce9c08a47"
                    .to_owned()
            ),
            (
                23,
                "0023_action_decision_risk_issue_h2a_lower_classification_replay_authority"
                    .to_owned(),
                "sha256:968d33c5f61db97a809acfb01fc2569520585d1638400981526e715acf9508a3"
                    .to_owned()
            ),
            (
                24,
                "0024_risk_h2a_lower_classification_prepare_replay_authority".to_owned(),
                "sha256:e605cc70db1ff4fac8e051a5841d10701a945363c921c4de21f49398e2111e6a"
                    .to_owned()
            ),
            (
                25,
                "0025_risk_h2a_lower_classification_execute_replay_authority".to_owned(),
                "sha256:abd6ef0575c8ac983a4ac107563d77ca6c55fa0ae2d91f2a5b82c4fd85623519"
                    .to_owned()
            ),
            (
                26,
                "0026_decision_h2a_lower_classification_prepare_replay_authority".to_owned(),
                "sha256:c1e1bb593f33ef12c4dfffaa5d643414b5ae40aca6dc00ada46162442caf010a"
                    .to_owned()
            ),
            (
                27,
                "0027_decision_h2a_lower_classification_execute_replay_authority".to_owned(),
                "sha256:86f7963ef0f337f5a6e84acc7cb093b62e8e5ab54d6c7cd39d011cf740cbb0a3"
                    .to_owned()
            ),
            (
                28,
                "0028_issue_h2a_lower_classification_prepare_replay_authority".to_owned(),
                "sha256:babab2989a631d593f201dc777c319c9cc22c79ef40fb72fe83751cd52712dcb"
                    .to_owned()
            ),
            (
                29,
                "0029_issue_h2a_lower_classification_execute_replay_authority".to_owned(),
                "sha256:15d8617792896903c435b50f4f676a75912e5def65b69353134b682785592dee"
                    .to_owned()
            ),
            (
                30,
                "0030_action_h2a_lower_classification_prepare_replay_authority".to_owned(),
                "sha256:d9fb6f1d5e988fe363b1aa9fc0e9e3739016f1a83054a7f80f1cfc1698e74138"
                    .to_owned()
            ),
            (
                31,
                "0031_action_h2a_lower_classification_execute_replay_authority".to_owned(),
                "sha256:b28be312b064dfecf4643e9ce8a3b939784647560ab8402c3e6979e7df25370f"
                    .to_owned()
            ),
            (
                32,
                "0032_delivery_h2a_lower_classification_prepared_intent_kinds".to_owned(),
                "sha256:459ffdffc141ba4cb31648f1b00ce7aa0e330ddc3129a69930607cd289009c20"
                    .to_owned()
            ),
            (
                33,
                "0033_delivery_h2a_lower_classification_prepare_replay_authority".to_owned(),
                "sha256:67b62b4d9a3249d6b0bfcce1157febc5c455129667b5ab8c62581571d9d08538"
                    .to_owned()
            ),
            (
                34,
                "0034_delivery_h2a_lower_classification_execute_replay_authority".to_owned(),
                "sha256:9cb91a7143affdd794af1317a9f89e832d165a4dc0974eff4e35d740610b0c27"
                    .to_owned()
            ),
            (
                35,
                "0035_delivery_h2a_lower_classification_execute_result_snapshot".to_owned(),
                "sha256:35cb22294066c599f5f8ec84ca1611e7cd36361b9f598ee22988146ca5c7d634"
                    .to_owned()
            ),
            (
                36,
                "0036_action_decision_triggered_replay_authority".to_owned(),
                "sha256:f808296cf5953defbd32ab2d5ff32b019d6ce6f7980907c088a7fa69b4b0d70a"
                    .to_owned()
            ),
            (
                37,
                "0037_decision_h2a_supersede_replay_authority".to_owned(),
                "sha256:529a576f909ac431139476d32dddef84866a58989a4b5fa06d01ed9d31554b6e"
                    .to_owned()
            ),
            (
                38,
                "0038_action_h2a_complete_evidence_snapshot".to_owned(),
                "sha256:c806c87b86e502454f6e77c07b2833704cb811d9cb1568835cd8641da18aafcb"
                    .to_owned()
            ),
            (
                39,
                "0039_evidence_relocation_replay_authority".to_owned(),
                "sha256:6654135ba5f780fbf9ebfcdbb59997c9e91cb15bfecd2c77d14917d7c3f89794"
                    .to_owned()
            ),
            (
                40,
                "0040_evidence_supersession_h2a_replay_authority".to_owned(),
                "sha256:2ce78d3ceb6576f2f3ed9527d98cf143023cab148bf9fea5e73c01971c4f797f"
                    .to_owned()
            ),
            (
                41,
                "0041_managed_projection_rebuild_h2a_persistence".to_owned(),
                "sha256:1672216c89a08abe89657240561771c941e8ee0bee9d3b849d5ff3a499dd2823"
                    .to_owned()
            ),
            (
                42,
                "0042_managed_projection_h1_auto_publication".to_owned(),
                "sha256:16c7f0bd1a200bca3bf3a6af824df3823844430d212b271c3ead6bf36046ae6b"
                    .to_owned()
            ),
            (
                43,
                "0043_evidence_observed_unpinned_verification".to_owned(),
                "sha256:e46ae813773c112fb7a7edbd4e5ea4cdac3bb8fa11f6f18ef85c6d0114a987ea"
                    .to_owned()
            ),
            (
                44,
                "0044_evidence_fingerprint_pin_replay_authority".to_owned(),
                "sha256:6bbcc105a74e21e46e99a8576ad9163b9aa8b470e216e87e42db04cda4ad3541"
                    .to_owned()
            ),
            (
                45,
                "0045_work_management_rejection_and_evidence_binding".to_owned(),
                "sha256:627c6d4fb88191651d671e54379797553764d3ec300e90d1069cacaf5c51f008"
                    .to_owned()
            ),
            (
                46,
                "0046_risk_issue_prepared_rejection".to_owned(),
                "sha256:9eb8562ae45a5d685f7ecc84142d9ee5bcbb113b77cea3d7013406b9d5635c28"
                    .to_owned()
            ),
            (
                47,
                "0047_record_entry_foundation".to_owned(),
                "sha256:ec9f56c823e333aeea1d95f5f592c0eeae4c21914d91cdf2101ac18254163f16"
                    .to_owned()
            ),
            (
                48,
                "0048_evidence_from_file".to_owned(),
                "sha256:837033a7b0772d6fca00fa3cc76490eb13b4013aebf066281172f6e89bcb8793"
                    .to_owned()
            ),
        ]
    );

    let names = connection
        .prepare("SELECT name FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY name")
        .expect("schema inventory query must prepare")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("schema inventory must query")
        .collect::<Result<Vec<_>, _>>()
        .expect("schema inventory must decode");
    assert_eq!(names, expected_objects());
}

fn expected_objects() -> Vec<String> {
    let mut values = [
        "action_completion_evidence",
        "action_requests",
        "action_transitions",
        "actions",
        "aggregate_registry",
        "approval_receipts",
        "audit_effects",
        "audit_events",
        "decision_requests",
        "decision_command_create_requests",
        "decision_command_transition_requests",
        "decision_h2a_command_execute_resolves",
        "decision_h2a_command_execute_binding_insert",
        "decision_h2a_command_execute_binding_update",
        "decision_h2a_command_prepare_resolves",
        "decision_reject_prepared_command_results",
        "decision_reject_prepared_command_results_contiguous_v45",
        "decision_h2a_command_prepare_supersedes",
        "decision_h2a_command_execute_supersedes",
        "decision_h2a_supersede_execute_binding_insert",
        "decision_h2a_supersede_execute_binding_update",
        "decision_h2a_command_supersede_execute_binding_insert",
        "decision_h2a_command_supersede_execute_binding_update",
        "decision_h2a_correlation_anchor_immutable",
        "decision_h2a_correlation_anchor_no_delete",
        "decision_h2a_correlation_anchors",
        "decision_h2a_execute_binding_insert",
        "decision_h2a_execute_binding_update",
        "decision_h2a_lower_classification_prepare_replay_operations",
        "decision_h2a_lower_classification_command_prepares",
        "decision_h2a_lower_classification_prepare_replay_audits",
        "decision_h2a_lower_classification_execute_replay_operations",
        "decision_h2a_lower_classification_command_executes",
        "decision_h2a_lower_classification_execute_replay_audits",
        "ledger_idempotency_claim_decision_h2a_lower_classification_prepare",
        "decision_h2a_lower_classification_prepare_replay_operations_ordinal_immutable",
        "idx_decision_h2a_lower_classification_prepare_replay_ordinal",
        "idx_decision_h2a_lower_classification_prepare_replay_correlation",
        "ledger_idempotency_claim_decision_h2a_lower_classification_execute",
        "decision_h2a_lower_classification_execute_replay_operations_ordinal_immutable",
        "idx_decision_h2a_lower_classification_execute_replay_ordinal",
        "idx_decision_h2a_lower_classification_execute_replay_correlation",
        "decision_replay_operations_contiguous_v45",
        "decision_h2a_replay_operations_contiguous_v45",
        "decision_h2a_lower_classification_prepare_replay_operations_contiguous_v45",
        "decision_h2a_lower_classification_execute_replay_operations_contiguous_v45",
        "decision_h2a_replay_audits",
        "decision_h2a_replay_correlation_immutable",
        "decision_h2a_replay_operations",
        "decision_h2a_support_evidence_snapshots",
        "decision_h2a_replay_operations_ordinal_immutable",
        "decision_h2a_prepared_intent_kind_insert",
        "decision_h2a_prepared_intent_kind_update",
        "decision_replay_audits",
        "decision_replay_operations",
        "decision_replay_operations_ordinal_immutable",
        "decision_request_transitions",
        "decision_resulting_action_requests",
        "decisions",
        "delivery_derived_milestone_mutations",
        "delivery_command_results",
        "delivery_idempotency_outcome_audits",
        "delivery_idempotency_outcomes",
        "evidence_link_command_results",
        "evidence_links",
        "evidence_reference_command_results",
        "evidence_references",
        "evidence_relocation_command_results",
        "evidence_fingerprint_pin_command_results",
        "evidence_verification_command_results",
        "evidence_h2a_command_prepare_supersessions",
        "evidence_h2a_command_execute_supersessions",
        "evidence_h2a_supersession_prepare_replay_operations",
        "evidence_h2a_supersession_execute_replay_operations",
        "evidence_h2a_supersession_execute_replay_audits",
        "evidence_reference_supersessions",
        "idempotency_outcomes",
        "initiatives",
        "issue_evidence",
        "issue_command_creates",
        "issue_reopen_history",
        "issue_replay_audits",
        "issue_replay_operations",
        "issue_support_history",
        "issues",
        "kpi_definitions",
        "kpi_observations",
        "ledger_metadata",
        "ledger_idempotency_claims",
        "milestones",
        "operation_effects",
        "operations",
        "portfolio_command_results",
        "portfolio_derived_kpi_observation_mutations",
        "portfolio_h2a_command_execute_lower_classifications",
        "portfolio_h2a_command_prepare_lower_classifications",
        "portfolio_h2a_execute_replay_audits",
        "portfolio_h2a_execute_replay_operations",
        "portfolio_h2a_prepare_replay_audits",
        "portfolio_h2a_prepare_replay_operations",
        "product_h2a_command_execute_lower_classifications",
        "product_h2a_command_prepare_lower_classifications",
        "product_h2a_execute_replay_audits",
        "product_h2a_execute_replay_operations",
        "product_h2a_prepare_replay_audits",
        "product_h2a_prepare_replay_operations",
        "roadmap_h2a_command_execute_lower_classifications",
        "roadmap_h2a_command_prepare_lower_classifications",
        "roadmap_h2a_execute_replay_audits",
        "roadmap_h2a_execute_replay_operations",
        "roadmap_h2a_prepare_replay_audits",
        "roadmap_h2a_prepare_replay_operations",
        "kpi_h2a_command_execute_lower_classifications",
        "kpi_h2a_command_prepare_lower_classifications",
        "kpi_h2a_execute_replay_audits",
        "kpi_h2a_execute_replay_operations",
        "kpi_h2a_prepare_replay_audits",
        "kpi_h2a_prepare_replay_operations",
        "kpi_observation_h2a_command_execute_lower_classifications",
        "kpi_observation_h2a_command_prepare_lower_classifications",
        "kpi_observation_h2a_execute_replay_audits",
        "kpi_observation_h2a_execute_replay_operations",
        "kpi_observation_h2a_prepare_replay_audits",
        "kpi_observation_h2a_prepare_replay_operations",
        "portfolio_idempotency_outcome_audits",
        "portfolio_idempotency_outcomes",
        "portfolios",
        "prepared_intent_classification_sources",
        "prepared_intent_effects",
        "prepared_intent_recovery_evidence",
        "prepared_intent_targets",
        "prepared_intents",
        "prepared_evidence_classifications",
        "prepared_evidence_supersession_payloads",
        "prepared_evidence_supersession_links",
        "prepared_incomplete_downstream",
        "prepared_removal_endpoints",
        "prepared_removal_payloads",
        "prepared_resulting_action_requests",
        "prepared_work_management_payloads",
        "products",
        "projects",
        "relationships",
        "relationship_endpoints",
        "relationship_h2b_command_results",
        "relationship_link_command_endpoints",
        "relationship_link_command_results",
        "relationship_link_result_endpoints",
        "relationship_replay_audits",
        "relationship_replay_operations",
        "relationship_replay_tombstones",
        "relationship_stakeholder_command_results",
        "risk_issue_links",
        "risk_command_creates",
        "risk_h2a_command_execute_closes",
        "risk_h2a_command_execute_occurrences",
        "risk_h2a_command_prepare_closes",
        "risk_h2a_command_prepare_occurrences",
        "risk_h2a_correlation_anchor_immutable",
        "risk_h2a_correlation_anchor_no_delete",
        "risk_h2a_correlation_anchors",
        "risk_h2a_replay_audits",
        "risk_h2a_replay_correlation_immutable",
        "risk_h2a_replay_operations",
        "risk_h2a_v11_terminal_denials",
        "risk_h2a_v11_terminal_denials_append_only_delete",
        "risk_h2a_v11_terminal_denials_append_only_update",
        "risk_h2a_v11_terminal_denials_operation_audit_code_insert",
        "risk_h2a_v11_terminal_denials_topology_insert",
        "risk_h2a_v13_command_prepare_closes",
        "risk_h2a_v13_command_prepare_occurrences",
        "risk_h2a_v13_replay_audits",
        "risk_h2a_v13_replay_operations",
        "risk_h2a_v14_command_execute_closes",
        "risk_h2a_v14_command_execute_occurrences",
        "risk_h2a_v14_replay_audits",
        "risk_h2a_v14_replay_operations",
        "risk_h2a_lower_classification_command_executes",
        "risk_h2a_lower_classification_command_prepares",
        "risk_h2a_lower_classification_execute_replay_audits",
        "risk_h2a_lower_classification_execute_replay_operations",
        "risk_h2a_lower_classification_prepare_replay_audits",
        "risk_h2a_lower_classification_prepare_replay_operations",
        "risk_h2a_lower_classification_terminal_denials",
        "issue_h2a_prepare_replay_operations",
        "issue_h2a_command_prepare_resolves",
        "issue_h2a_command_prepare_closes",
        "issue_h2a_command_prepare_reopens",
        "issue_h2a_support_evidence_snapshots",
        "issue_h2a_execute_replay_operations",
        "issue_h2a_command_execute_resolves",
        "issue_h2a_command_execute_closes",
        "issue_h2a_command_execute_reopens",
        "issue_h2a_execute_replay_audits",
        "issue_h2a_lower_classification_prepare_replay_operations",
        "issue_h2a_lower_classification_command_prepares",
        "issue_h2a_lower_classification_prepare_replay_audits",
        "issue_h2a_lower_classification_execute_replay_operations",
        "issue_h2a_lower_classification_command_executes",
        "issue_h2a_lower_classification_execute_replay_audits",
        "risk_replay_audits",
        "risk_replay_operations",
        "record_id_reservations",
        "record_id_reservations_immutable",
        "record_id_reservations_no_delete",
        "idx_evidence_references_vault_relative_path",
        "idx_evidence_references_fingerprint",
        "risk_response_replay_operations",
        "risk_response_command_updates",
        "risk_response_replay_audits",
        "idx_risk_response_replay_ordinal",
        "idx_risk_response_replay_correlation",
        "idx_risk_response_replay_risk",
        "ledger_idempotency_claim_risk_response",
        "risk_response_replay_operations_contiguous_insert",
        "risk_response_replay_operations_ordinal_immutable",
        "risks",
        "roadmaps",
        "schema_migrations",
        "stakeholders",
        "support_evidence",
        "support_judgments",
        "support_witnesses",
        "action_command_create_requests",
        "ledger_idempotency_claim_action",
        "ledger_idempotency_claim_action_reject_prepared",
        "ledger_idempotency_claim_risk_reject_prepared",
        "ledger_idempotency_claim_issue_reject_prepared",
        "action_reject_prepared_ordinal_immutable",
        "action_reject_prepared_binding_insert",
        "risk_reject_prepared_binding_insert",
        "risk_reject_prepared_binding_update",
        "issue_reject_prepared_binding_insert",
        "issue_reject_prepared_binding_update",
        "action_reject_prepared_binding_update",
        "ledger_idempotency_claim_decision",
        "ledger_idempotency_claim_decision_reject_prepared",
        "decision_reject_prepared_ordinal_immutable",
        "decision_reject_prepared_binding_insert",
        "decision_reject_prepared_binding_update",
        "ledger_idempotency_claim_decision_h2a",
        "ledger_idempotency_claim_evidence_link",
        "ledger_idempotency_claim_evidence_reference",
        "ledger_idempotency_claim_evidence_relocation",
        "ledger_idempotency_claim_evidence_fingerprint_pin",
        "ledger_idempotency_claim_evidence_verification",
        "ledger_idempotency_claim_evidence_h2a_supersession_prepare",
        "ledger_idempotency_claim_evidence_h2a_supersession_execute",
        "ledger_idempotency_claim_operation",
        "ledger_idempotency_claim_outcome",
        "ledger_idempotency_claim_portfolio_h2a_execute",
        "portfolio_h2a_execute_replay_operations_contiguous_insert",
        "portfolio_h2a_execute_replay_operations_ordinal_immutable",
        "idx_portfolio_h2a_execute_replay_correlation",
        "idx_portfolio_h2a_execute_replay_ordinal",
        "ledger_idempotency_claim_portfolio_h2a_prepare",
        "portfolio_h2a_prepare_replay_operations_contiguous_insert",
        "portfolio_h2a_prepare_replay_operations_ordinal_immutable",
        "idx_portfolio_h2a_prepare_replay_correlation",
        "idx_portfolio_h2a_prepare_replay_ordinal",
        "ledger_idempotency_claim_product_h2a_execute",
        "product_h2a_execute_replay_operations_contiguous_insert",
        "product_h2a_execute_replay_operations_ordinal_immutable",
        "idx_product_h2a_execute_replay_correlation",
        "idx_product_h2a_execute_replay_ordinal",
        "ledger_idempotency_claim_product_h2a_prepare",
        "product_h2a_prepare_replay_operations_contiguous_insert",
        "product_h2a_prepare_replay_operations_ordinal_immutable",
        "idx_product_h2a_prepare_replay_correlation",
        "idx_product_h2a_prepare_replay_ordinal",
        "ledger_idempotency_claim_roadmap_h2a_execute",
        "roadmap_h2a_execute_replay_operations_contiguous_insert",
        "roadmap_h2a_execute_replay_operations_ordinal_immutable",
        "idx_roadmap_h2a_execute_replay_correlation",
        "idx_roadmap_h2a_execute_replay_ordinal",
        "ledger_idempotency_claim_roadmap_h2a_prepare",
        "roadmap_h2a_prepare_replay_operations_contiguous_insert",
        "roadmap_h2a_prepare_replay_operations_ordinal_immutable",
        "idx_roadmap_h2a_prepare_replay_correlation",
        "idx_roadmap_h2a_prepare_replay_ordinal",
        "ledger_idempotency_claim_kpi_h2a_execute",
        "kpi_h2a_execute_replay_operations_contiguous_insert",
        "kpi_h2a_execute_replay_operations_ordinal_immutable",
        "idx_kpi_h2a_execute_replay_correlation",
        "idx_kpi_h2a_execute_replay_ordinal",
        "ledger_idempotency_claim_kpi_h2a_prepare",
        "kpi_h2a_prepare_replay_operations_contiguous_insert",
        "kpi_h2a_prepare_replay_operations_ordinal_immutable",
        "idx_kpi_h2a_prepare_replay_correlation",
        "idx_kpi_h2a_prepare_replay_ordinal",
        "ledger_idempotency_claim_kpi_observation_h2a_execute",
        "kpi_observation_h2a_execute_replay_operations_contiguous_insert",
        "kpi_observation_h2a_execute_replay_operations_ordinal_immutable",
        "idx_kpi_observation_h2a_execute_replay_correlation",
        "idx_kpi_observation_h2a_execute_replay_ordinal",
        "ledger_idempotency_claim_kpi_observation_h2a_prepare",
        "kpi_observation_h2a_prepare_replay_operations_contiguous_insert",
        "kpi_observation_h2a_prepare_replay_operations_ordinal_immutable",
        "idx_kpi_observation_h2a_prepare_replay_correlation",
        "idx_kpi_observation_h2a_prepare_replay_ordinal",
        "ledger_idempotency_claim_relationship",
        "ledger_idempotency_claim_risk",
        "ledger_idempotency_claim_risk_h2a",
        "ledger_idempotency_claim_risk_h2a_v13",
        "risk_h2a_v13_replay_operations_contiguous_insert",
        "risk_h2a_v13_replay_operations_ordinal_immutable",
        "idx_risk_h2a_v13_replay_correlation",
        "idx_risk_h2a_v13_replay_ordinal",
        "ledger_idempotency_claim_risk_h2a_v14",
        "risk_h2a_v14_replay_operations_contiguous_insert",
        "risk_h2a_v14_replay_operations_ordinal_immutable",
        "idx_risk_h2a_v14_replay_correlation",
        "idx_risk_h2a_v14_replay_ordinal",
        "ledger_idempotency_claim_risk_h2a_lower_classification_execute",
        "risk_h2a_lower_classification_execute_replay_operations_contiguous_insert",
        "risk_h2a_lower_classification_execute_replay_operations_ordinal_immutable",
        "idx_risk_h2a_lower_classification_execute_replay_correlation",
        "idx_risk_h2a_lower_classification_execute_replay_ordinal",
        "ledger_idempotency_claim_risk_h2a_lower_classification_prepare",
        "risk_h2a_lower_classification_prepare_replay_operations_contiguous_insert",
        "risk_h2a_lower_classification_prepare_replay_operations_ordinal_immutable",
        "idx_risk_h2a_lower_classification_prepare_replay_correlation",
        "idx_risk_h2a_lower_classification_prepare_replay_ordinal",
        "risk_h2a_lower_classification_terminal_denials_topology_insert",
        "risk_h2a_lower_classification_terminal_denials_append_only_update",
        "risk_h2a_lower_classification_terminal_denials_append_only_delete",
        "risk_h2a_lower_classification_terminal_denials_operation_audit_code_insert",
        "ledger_idempotency_claim_issue_h2a_prepare",
        "issue_h2a_prepare_replay_operations_contiguous_insert",
        "issue_h2a_prepare_replay_operations_ordinal_immutable",
        "idx_issue_h2a_prepare_replay_ordinal",
        "idx_issue_h2a_prepare_replay_correlation",
        "idx_issue_h2a_support_evidence_snapshot_evidence",
        "ledger_idempotency_claim_issue_h2a_execute",
        "issue_h2a_execute_replay_operations_contiguous_insert",
        "issue_h2a_execute_replay_operations_ordinal_immutable",
        "idx_issue_h2a_execute_replay_ordinal",
        "idx_issue_h2a_execute_replay_correlation",
        "evidence_h2a_supersession_prepare_replay_operations_contiguous_insert",
        "evidence_h2a_supersession_prepare_replay_operations_ordinal_immutable",
        "idx_evidence_h2a_supersession_prepare_replay_ordinal",
        "idx_evidence_h2a_supersession_prepare_replay_correlation",
        "evidence_h2a_supersession_execute_replay_operations_contiguous_insert",
        "evidence_h2a_supersession_execute_replay_operations_ordinal_immutable",
        "idx_evidence_h2a_supersession_execute_replay_ordinal",
        "idx_evidence_h2a_supersession_execute_replay_correlation",
        "ledger_idempotency_claim_issue_h2a_lower_classification_prepare",
        "issue_h2a_lower_classification_prepare_replay_operations_contiguous_insert",
        "issue_h2a_lower_classification_prepare_replay_operations_ordinal_immutable",
        "idx_issue_h2a_lower_classification_prepare_replay_ordinal",
        "idx_issue_h2a_lower_classification_prepare_replay_correlation",
        "ledger_idempotency_claim_issue_h2a_lower_classification_execute",
        "issue_h2a_lower_classification_execute_replay_operations_contiguous_insert",
        "issue_h2a_lower_classification_execute_replay_operations_ordinal_immutable",
        "idx_issue_h2a_lower_classification_execute_replay_ordinal",
        "idx_issue_h2a_lower_classification_execute_replay_correlation",
        "action_command_execute_accepts",
        "action_command_execute_accept_binding_insert",
        "action_command_execute_accept_binding_update",
        "action_command_execute_actions",
        "action_command_execute_action_binding_insert",
        "action_command_execute_action_binding_update",
        "action_command_link_completion_evidence",
        "action_command_prepare_accepts",
        "action_reject_prepared_command_results",
        "action_reject_prepared_command_results_contiguous_v45",
        "risk_reject_prepared_command_results",
        "risk_reject_prepared_command_results_contiguous_insert",
        "risk_reject_prepared_command_results_ordinal_immutable",
        "issue_reject_prepared_command_results",
        "issue_reject_prepared_command_results_contiguous_insert",
        "issue_reject_prepared_command_results_ordinal_immutable",
        "action_command_prepare_cancels",
        "action_command_prepare_completes",
        "action_command_prepare_reopens",
        "action_command_start_actions",
        "action_command_transition_requests",
        "action_discarded_prepared_intents",
        "action_replay_audits",
        "action_replay_error_extensions",
        "action_replay_error_params",
        "action_replay_execute_accept_binding_insert",
        "action_replay_execute_accept_binding_update",
        "action_replay_execute_action_binding_insert",
        "action_replay_execute_action_binding_update",
        "action_replay_operations",
        "action_replay_terminal_shape_insert",
        "action_replay_terminal_shape_update",
        "action_h2a_lower_classification_prepare_replay_operations",
        "action_h2a_lower_classification_command_prepares",
        "action_h2a_lower_classification_prepare_replay_audits",
        "action_h2a_lower_classification_execute_replay_operations",
        "action_h2a_lower_classification_command_executes",
        "action_h2a_lower_classification_execute_replay_audits",
        "ledger_idempotency_claim_action_h2a_lower_classification_prepare",
        "ledger_idempotency_claim_action_h2a_lower_classification_execute",
        "action_h2a_lower_classification_prepare_replay_operations_contiguous_v45",
        "action_h2a_lower_classification_prepare_replay_operations_ordinal_immutable",
        "action_h2a_lower_classification_execute_replay_operations_contiguous_v45",
        "action_h2a_lower_classification_execute_replay_operations_ordinal_immutable",
        "idx_action_h2a_lower_classification_prepare_replay_ordinal",
        "idx_action_h2a_lower_classification_prepare_replay_correlation",
        "idx_action_h2a_lower_classification_execute_replay_ordinal",
        "idx_action_h2a_lower_classification_execute_replay_correlation",
        "idx_action_command_create_identity",
        "idx_action_command_execute_accept_identity",
        "idx_action_command_execute_accept_prepared_identity",
        "idx_action_command_execute_action_identity",
        "idx_action_command_execute_action_prepared_identity",
        "idx_action_command_link_completion_evidence_identity",
        "idx_action_command_prepare_accept_identity",
        "idx_action_command_prepare_cancel_identity",
        "idx_action_command_prepare_complete_identity",
        "idx_action_command_prepare_reopen_identity",
        "idx_action_command_start_action_identity",
        "idx_action_command_transition_identity",
        "idx_actions_due_at",
        "action_replay_operations_contiguous_v45",
        "action_replay_operations_ordinal_immutable",
        "idx_audit_events_occurred_at",
        "idx_issues_state",
        "idx_kpi_observations_kpi_observed",
        "idx_operations_correlation",
        "idx_prepared_intents_expires_at",
        "idx_relationship_endpoints_target",
        "idx_relationship_h2b_command_identity",
        "idx_relationship_replay_identity",
        "idx_risk_h2a_replay_correlation",
        "idx_risk_h2a_replay_ordinal",
        "idx_risks_review_at",
        "idx_action_replay_correlation",
        "idx_action_replay_discarded_terminal_identity",
        "idx_action_replay_ordinal",
        "idx_decision_h2a_replay_correlation",
        "idx_decision_h2a_replay_ordinal",
        "idx_decision_h2a_support_evidence_snapshot_evidence",
        "delivery_h2a_prepare_replay_operations",
        "delivery_h2a_command_prepares",
        "delivery_h2a_prepare_replay_audits",
        "ledger_idempotency_claim_delivery_h2a_prepare",
        "delivery_h2a_prepare_replay_operations_contiguous_insert",
        "delivery_h2a_prepare_replay_operations_ordinal_immutable",
        "idx_delivery_h2a_prepare_replay_ordinal",
        "idx_delivery_h2a_prepare_replay_correlation",
        "delivery_h2a_execute_replay_operations",
        "delivery_h2a_command_executes",
        "delivery_h2a_execute_replay_audits",
        "ledger_idempotency_claim_delivery_h2a_execute",
        "idx_delivery_h2a_execute_replay_ordinal",
        "idx_delivery_h2a_execute_replay_correlation",
        "action_decision_replay_operations",
        "action_command_create_from_decisions",
        "action_command_mark_request_superseded_premises",
        "action_command_mark_action_superseded_premises",
        "action_decision_replay_audits",
        "ledger_idempotency_claim_action_decision",
        "action_decision_replay_operations_contiguous_v45",
        "action_decision_replay_operations_ordinal_immutable",
        "idx_action_decision_replay_ordinal",
        "idx_action_decision_replay_correlation",
        "action_h2a_support_evidence_snapshots",
        "idx_action_h2a_support_evidence_snapshot_evidence",
        "idx_action_reject_prepared_correlation",
        "idx_risk_reject_prepared_ordinal",
        "idx_risk_reject_prepared_correlation",
        "idx_issue_reject_prepared_ordinal",
        "idx_issue_reject_prepared_correlation",
        "idx_decision_reject_prepared_correlation",
        // V41 managed-projection rebuild H2a persistence.
        "prepared_rebuild_managed_projections_payloads",
        "prepared_rebuild_managed_projections_changes",
        "prepared_rebuild_managed_projections_changes_contiguous_insert",
        "prepared_rebuild_managed_projections_changes_ordinal_immutable",
        "rebuild_managed_projections_prepare_commands",
        "rebuild_managed_projections_prepare_replay_operations",
        "ledger_idempotency_claim_rebuild_managed_projections_prepare",
        "rebuild_managed_projections_prepare_replay_contiguous_insert",
        "rebuild_managed_projections_prepare_replay_ordinal_immutable",
        "idx_rebuild_managed_projections_prepare_replay_ordinal",
        "idx_rebuild_managed_projections_prepare_replay_correlation",
        "rebuild_managed_projections_execute_commands",
        "rebuild_managed_projections_execute_replay_operations",
        "rebuild_managed_projections_operation_items",
        "rebuild_managed_projections_operation_items_contiguous_insert",
        "rebuild_managed_projections_execute_replay_audits",
        "ledger_idempotency_claim_rebuild_managed_projections_execute",
        "rebuild_managed_projections_execute_replay_contiguous_insert",
        "rebuild_managed_projections_execute_replay_ordinal_immutable",
        "idx_rebuild_managed_projections_execute_replay_ordinal",
        "idx_rebuild_managed_projections_execute_replay_correlation",
        "idx_rebuild_managed_projections_execute_replay_status",
        "projection_manifest_generations",
        "projection_manifest_entries",
        "projection_manifest_head",
        // V42 adds only one new object: the authorization index.
        "idx_rebuild_managed_projections_execute_replay_authorization",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    values.sort();
    values
}

#[test]
fn relationship_replay_schema_is_normalized_without_blob_or_json_authority() {
    let ledger = SyntheticLedger::new("relationship-normalized");
    drop(SqliteProductLedger::open(&ledger.0).expect("fixture must initialize"));
    let connection = Connection::open(&ledger.0).expect("fixture must open");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys must be explicit per connection");
    let tables = [
        "relationship_replay_operations",
        "relationship_replay_audits",
        "relationship_stakeholder_command_results",
        "relationship_link_command_results",
        "relationship_link_command_endpoints",
        "relationship_link_result_endpoints",
        "relationship_h2b_command_results",
        "relationship_replay_tombstones",
    ];
    for table in tables {
        let forbidden = connection
            .query_row(
                "SELECT count(*) FROM pragma_table_xinfo(?1) WHERE upper(type) IN ('BLOB','ANY')",
                [table],
                |row| row.get::<_, i64>(0),
            )
            .expect("typed column inventory must be readable");
        assert_eq!(forbidden, 0, "{table} must remain normalized and typed");
        let strict = connection
            .query_row(
                "SELECT strict FROM pragma_table_list WHERE name=?1",
                [table],
                |row| row.get::<_, i64>(0),
            )
            .expect("table strictness must be inspectable");
        assert_eq!(strict, 1, "{table} must remain STRICT");
    }
    let foreign_key_errors = connection
        .prepare("PRAGMA foreign_key_check")
        .expect("foreign-key check must prepare")
        .query_map([], |_| Ok(()))
        .expect("foreign-key check must execute")
        .count();
    assert_eq!(
        foreign_key_errors, 0,
        "fresh canonical schema must be coherent"
    );
}

#[test]
fn terminal_h2b_history_does_not_require_resurrecting_the_removed_relationship() {
    let ledger = SyntheticLedger::new("relationship-terminal-history");
    drop(SqliteProductLedger::open(&ledger.0).expect("fixture must initialize"));
    let connection = Connection::open(&ledger.0).expect("fixture must open");
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON;
             BEGIN IMMEDIATE;
             INSERT INTO prepared_intents(
                 id,contract_version,intent_kind,payload_digest,classification,policy,
                 cancellation_policy,authority,confirmation_challenge,expires_at,created_at,consumed_at
             ) VALUES(
                 'prepared-terminal-history',1,'relationship.remove',
                 '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef',
                 'internal','allowed','not_cancellable_after_submit','head_of_products',
                 'REMOVE prepared-terminal-history',301000,1000,1100
             );
             INSERT INTO prepared_removal_payloads(
                 prepared_intent_id,relationship_id,relationship_version,relationship_kind,purpose
             ) VALUES(
                 'prepared-terminal-history','relationship-already-removed',1,
                 'portfolio_product','none'
             );
             INSERT INTO prepared_intent_recovery_evidence(
                 prepared_intent_id,recovery_evidence_id,name,verified_at,relationship_id,compatible
             ) VALUES(
                 'prepared-terminal-history','recovery-terminal-history','synthetic recovery',900,
                 'relationship-already-removed',1
             );
             COMMIT;",
        )
        .expect(
            "typed H2b history must remain durable after current relationship authority is removed",
        );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM relationships", [], |row| row
                .get::<_, i64>(0))
            .expect("current relationship inventory"),
        0,
        "durable history must not resurrect the removed relationship"
    );
}

#[test]
fn current_milestone_relationship_endpoint_retains_and_validates_parent_project() {
    let ledger = SyntheticLedger::new("relationship-milestone-parent");
    drop(SqliteProductLedger::open(&ledger.0).expect("fixture must initialize"));
    let connection = Connection::open(&ledger.0).expect("fixture must open");
    let columns = connection
        .prepare("SELECT name FROM pragma_table_xinfo('relationship_endpoints') ORDER BY cid")
        .expect("endpoint column inventory")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("endpoint columns must query")
        .collect::<Result<Vec<_>, _>>()
        .expect("endpoint columns must decode");
    assert!(
        columns.iter().any(|name| name == "parent_project_id"),
        "Milestone endpoint parentage must be durable in current relationship authority"
    );
    assert!(
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON;
                 BEGIN IMMEDIATE;
                 INSERT INTO aggregate_registry VALUES('project-parent','project',1,'internal',1,1);
                 INSERT INTO aggregate_registry VALUES('project-wrong','project',1,'internal',1,1);
                 INSERT INTO aggregate_registry VALUES('milestone-parented','milestone',1,'internal',1,1);
                 INSERT INTO aggregate_registry VALUES('stakeholder-parent-test','stakeholder',1,'internal',1,1);
                 INSERT INTO aggregate_registry VALUES('relationship-parent-test','relationship',1,'internal',1,1);
                 INSERT INTO projects VALUES('project-parent','Parent',1,2,'user_entered',NULL);
                 INSERT INTO projects VALUES('project-wrong','Wrong',1,2,'user_entered',NULL);
                 INSERT INTO milestones VALUES('milestone-parented','project-parent','Milestone','Verify',2,'user_entered',NULL);
                 INSERT INTO stakeholders VALUES('stakeholder-parent-test','Synthetic','person','user_entered',NULL);
                 INSERT INTO relationships VALUES('relationship-parent-test','stakeholder_subject','responsibility');
                 INSERT INTO relationship_endpoints(
                     relationship_id,ordinal,target_type,target_id,target_version,
                     target_classification,parent_project_id
                 ) VALUES(
                     'relationship-parent-test',0,'stakeholder','stakeholder-parent-test',1,
                     'internal',NULL
                 );
                 INSERT INTO relationship_endpoints(
                     relationship_id,ordinal,target_type,target_id,target_version,
                     target_classification,parent_project_id
                 ) VALUES(
                     'relationship-parent-test',1,'milestone','milestone-parented',1,
                     'internal','project-wrong'
                 );",
            )
            .is_err(),
        "a Milestone endpoint cannot claim a different parent Project"
    );
    connection
        .execute_batch("ROLLBACK;")
        .expect("failed parent insert remains rollback-safe");
}

#[test]
fn portfolio_stored_outcome_requires_a_typed_exact_pair_and_closed_mutation_topology() {
    let ledger = SyntheticLedger::new("portfolio-stored-outcome");
    drop(SqliteProductLedger::open(&ledger.0).expect("fixture must initialize"));
    let connection = Connection::open(&ledger.0).expect("fixture must open");
    connection.execute_batch("PRAGMA foreign_keys=ON;
        INSERT INTO aggregate_registry VALUES('kpi-p','kpi_definition',2,'restricted',1,4);
        INSERT INTO kpi_definitions VALUES('kpi-p','Synthetic KPI','Synthetic definition','Synthetic owner','Synthetic target','weekly','Synthetic source','synthetic_fixture','fixture-p');
        INSERT INTO aggregate_registry VALUES('observation-p','kpi_observation',2,'restricted',2,4);
        INSERT INTO kpi_observations VALUES('observation-p','kpi-p','42',3,'Synthetic observation source','synthetic_fixture','fixture-p');
        INSERT INTO audit_events VALUES('audit-primary-p',4,'head_of_products','portfolio','kpi.updated','kpi_definition','kpi-p','correlation-original-p','not_required','not_required','succeeded','complete');
        INSERT INTO audit_events VALUES('audit-derived-p',4,'policy_authorized_system','classification','kpi_observation.classification_inherited','kpi_observation','observation-p','correlation-original-p','not_required','not_required','succeeded','complete');
        INSERT INTO idempotency_outcomes VALUES('portfolio','update_kpi_definition','idempotency-p','digest-p','succeeded','kpi-p',4);
        BEGIN IMMEDIATE;
        INSERT INTO portfolio_idempotency_outcomes VALUES('portfolio','update_kpi_definition','idempotency-p','update_kpi_definition','correlation-original-p',0,'succeeded','kpi-p');
        INSERT INTO portfolio_command_results(namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_definition,command_owner,command_target,command_cadence,command_source,command_classification,result_kind,result_id,result_name,result_definition,result_owner,result_target,result_cadence,result_source,result_classification,result_provenance_kind,result_provenance_reference,result_version,result_created_at,result_updated_at) VALUES('portfolio','update_kpi_definition','idempotency-p','update_kpi_definition','kpi-p',1,'Synthetic KPI','Synthetic definition','Synthetic owner','Synthetic target','weekly','Synthetic source','restricted','kpi_definition','kpi-p','Synthetic KPI','Synthetic definition','Synthetic owner','Synthetic target','weekly','Synthetic source','restricted','synthetic_fixture','fixture-p',2,1,4);
        INSERT INTO portfolio_idempotency_outcome_audits VALUES('portfolio','update_kpi_definition','idempotency-p',0,'audit-primary-p');
        INSERT INTO portfolio_idempotency_outcome_audits VALUES('portfolio','update_kpi_definition','idempotency-p',1,'audit-derived-p');
        INSERT INTO portfolio_derived_kpi_observation_mutations VALUES('portfolio','update_kpi_definition','idempotency-p',0,'observation-p',1,2,'internal','restricted',2,4,'audit-derived-p');
        COMMIT;")
        .expect("normalized Portfolio replay must commit");

    let replay: (String, i64, String, String) = connection.query_row(
        "SELECT o.correlation_id,o.operation_ordinal,r.result_kind,
            (SELECT group_concat(audit_event_id,'|') FROM (SELECT audit_event_id FROM portfolio_idempotency_outcome_audits a WHERE a.namespace=o.namespace AND a.operation=o.operation AND a.idempotency_id=o.idempotency_id ORDER BY ordinal))
         FROM portfolio_idempotency_outcomes o JOIN portfolio_command_results r USING(namespace,operation,idempotency_id)
         WHERE o.idempotency_id='idempotency-p'",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))
        .expect("typed replay must decode");
    assert_eq!(
        replay,
        (
            "correlation-original-p".into(),
            0,
            "kpi_definition".into(),
            "audit-primary-p|audit-derived-p".into()
        )
    );

    assert!(connection.execute("INSERT INTO portfolio_idempotency_outcomes VALUES('portfolio','create_product','idempotency-p','create_product','cross-operation',1,'succeeded','kpi-p')", []).is_err());
    assert!(connection.execute("INSERT INTO portfolio_derived_kpi_observation_mutations VALUES('portfolio','update_kpi_definition','idempotency-p',1,'observation-p',2,4,'restricted','internal',4,3,'audit-derived-p')", []).is_err());
    for rejected in [
        "INSERT INTO portfolio_idempotency_outcome_audits VALUES('portfolio','update_kpi_definition','idempotency-p',2,'missing-audit')",
        "INSERT INTO portfolio_idempotency_outcome_audits VALUES('portfolio','update_kpi_definition','idempotency-p',2,'audit-primary-p')",
        "INSERT INTO portfolio_derived_kpi_observation_mutations VALUES('portfolio','create_kpi_definition','idempotency-p',1,'observation-p',1,2,'internal','restricted',2,4,'audit-derived-p')",
        "INSERT INTO portfolio_derived_kpi_observation_mutations VALUES('portfolio','update_kpi_definition','idempotency-p',1,'observation-p',1,2,'restricted','restricted',5,4,'audit-derived-p')",
    ] {
        assert!(connection.execute(rejected, []).is_err(), "must reject {rejected}");
    }
    assert!(connection.execute("UPDATE portfolio_derived_kpi_observation_mutations SET previous_classification='unclassified' WHERE idempotency_id='idempotency-p'", []).is_err(), "Unclassified cannot be lowered");
    assert!(connection.execute("UPDATE portfolio_derived_kpi_observation_mutations SET previous_updated_at=5,resulting_updated_at=4 WHERE idempotency_id='idempotency-p'", []).is_err(), "derived mutation time cannot move backward");
    connection.execute_batch("BEGIN IMMEDIATE; INSERT INTO idempotency_outcomes VALUES('portfolio','create_product','missing-child','digest','succeeded','product-x',5); INSERT INTO portfolio_idempotency_outcomes VALUES('portfolio','create_product','missing-child','create_product','correlation-x',1,'succeeded','product-x');").expect("deferred parent may stage");
    assert!(connection.execute_batch("COMMIT").is_err());
    connection
        .execute_batch("ROLLBACK")
        .expect("failed deferred commit must roll back");

    connection.execute_batch("BEGIN IMMEDIATE; INSERT INTO idempotency_outcomes VALUES('portfolio','create_product','mismatched-pair','digest','succeeded','product-x',5); INSERT INTO portfolio_idempotency_outcomes VALUES('portfolio','create_product','mismatched-pair','create_product','correlation-x',1,'succeeded','product-x'); INSERT INTO portfolio_command_results(namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_details,command_classification,command_provenance_kind,result_kind,result_id,result_name,result_details,result_classification,result_provenance_kind,result_version,result_created_at,result_updated_at) VALUES('portfolio','create_roadmap','mismatched-pair','create_roadmap','roadmap-x',NULL,'Roadmap','Details','internal','user_entered','roadmap','roadmap-x','Roadmap','Details','internal','user_entered',1,5,5);").expect("mismatched keys may stage");
    assert!(connection.execute_batch("COMMIT").is_err());
    connection
        .execute_batch("ROLLBACK")
        .expect("mismatch must roll back");

    connection.execute_batch("BEGIN IMMEDIATE; INSERT INTO idempotency_outcomes VALUES('portfolio','create_product','result-mismatch','digest','succeeded','different-product',5); INSERT INTO portfolio_idempotency_outcomes VALUES('portfolio','create_product','result-mismatch','create_product','correlation-x',1,'succeeded','different-product'); INSERT INTO portfolio_command_results(namespace,operation,idempotency_id,command_kind,command_target_id,command_name,command_details,command_classification,command_provenance_kind,result_kind,result_id,result_name,result_details,result_classification,result_provenance_kind,result_version,result_created_at,result_updated_at) VALUES('portfolio','create_product','result-mismatch','create_product','actual-product','Product','Details','internal','user_entered','product','actual-product','Product','Details','internal','user_entered',1,5,5);").expect("contradictory envelope may stage under deferred links");
    assert!(
        connection.execute_batch("COMMIT").is_err(),
        "generic result reference must equal typed result"
    );
    connection
        .execute_batch("ROLLBACK")
        .expect("result mismatch must roll back");

    assert!(connection.execute_batch("BEGIN IMMEDIATE; INSERT INTO idempotency_outcomes VALUES('portfolio','create_product','wrong-outcome','digest','denied','product-x',5); INSERT INTO portfolio_idempotency_outcomes VALUES('portfolio','create_product','wrong-outcome','create_product','correlation-x',1,'denied','product-x');").is_err(), "non-success Portfolio history is not a stored successful result");
    connection
        .execute_batch("ROLLBACK")
        .expect("wrong outcome must roll back");

    assert!(connection.execute("INSERT INTO portfolio_command_results(namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_details,command_classification,command_provenance_kind,result_kind,result_id,result_name,result_details,result_classification,result_provenance_kind,result_version,result_created_at,result_updated_at) VALUES('portfolio','create_product','malformed-topology','create_product','product-x',1,'Product','Details','internal','user_entered','product','product-x','Product','Details','internal','user_entered',2,5,5)",[]).is_err(), "create topology cannot carry update lineage");
    assert!(connection.execute("INSERT INTO portfolio_command_results(namespace,operation,idempotency_id,command_kind,command_target_id,command_value,command_observed_at,command_source,command_kpi_id,command_provenance_kind,result_kind,result_id,result_value,result_observed_at,result_source,result_kpi_id,result_classification,result_provenance_kind,result_version,result_created_at,result_updated_at) VALUES('portfolio','create_kpi_observation','missing-parent-command','create_kpi_observation','observation-x','1',5,'source','missing-kpi','user_entered','kpi_observation','observation-x','1',5,'source','missing-kpi','unclassified','user_entered',1,5,5)",[]).is_err(), "KPI parent must exist");

    let non_typed_columns: i64 = connection.query_row(
        "SELECT (SELECT count(*) FROM pragma_table_xinfo('portfolio_idempotency_outcomes') WHERE upper(type) IN ('BLOB','JSON'))+(SELECT count(*) FROM pragma_table_xinfo('portfolio_command_results') WHERE upper(type) IN ('BLOB','JSON'))+(SELECT count(*) FROM pragma_table_xinfo('portfolio_idempotency_outcome_audits') WHERE upper(type) IN ('BLOB','JSON'))+(SELECT count(*) FROM pragma_table_xinfo('portfolio_derived_kpi_observation_mutations') WHERE upper(type) IN ('BLOB','JSON'))",
        [], |row| row.get(0)).expect("Portfolio replay column types must be inspectable");
    assert_eq!(non_typed_columns, 0);
}

fn artifacts(path: &Path) -> Vec<Option<Vec<u8>>> {
    [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ]
    .iter()
    .map(|artifact| fs::read(artifact).ok())
    .collect()
}

fn assert_rejected_unchanged(path: &Path) {
    let before = artifacts(path);
    assert!(matches!(
        SqliteProductLedger::open(path),
        Err(LedgerOpenError::InvalidMetadata)
    ));
    assert_eq!(artifacts(path), before);
}

#[test]
fn altered_canonical_schema_is_rejected_without_changes() {
    let ledger = SyntheticLedger::new("altered");
    drop(SqliteProductLedger::open(&ledger.0).expect("fixture must initialize"));
    let connection = Connection::open(&ledger.0).expect("fixture must open");
    connection
        .execute_batch("DROP INDEX idx_actions_due_at")
        .expect("fixture mutation must succeed");
    drop(connection);
    let before = fs::read(&ledger.0).expect("fixture must be readable");

    let result = SqliteProductLedger::open(&ledger.0);

    assert!(matches!(result, Err(LedgerOpenError::InvalidMetadata)));
    assert_eq!(
        fs::read(&ledger.0).expect("fixture must remain readable"),
        before
    );
}

#[test]
fn schema_altered_after_open_is_refused_by_the_next_write_transaction() {
    // The expected schema is built once per process; the live schema must still be read on
    // every transaction, so a change made behind an open handle is caught at the next write.
    let ledger = SyntheticLedger::new("altered-after-open");
    let mut open = SqliteProductLedger::open(&ledger.0).expect("fixture must initialize");
    open.with_immediate_transaction(|_| Ok::<(), ()>(()))
        .expect("a canonical ledger must accept a transaction");
    let connection = Connection::open(&ledger.0).expect("fixture must open");
    connection
        .execute_batch("DROP INDEX idx_actions_due_at")
        .expect("fixture mutation must succeed");
    drop(connection);

    let result = open.with_immediate_transaction(|_| Ok::<(), ()>(()));

    assert!(matches!(
        result,
        Err(LedgerTransactionError::IncompatibleLedger)
    ));
}

#[test]
fn canonical_v1_reopens_without_mutating_database_or_sidecars() {
    let ledger = SyntheticLedger::new("reopen");
    drop(SqliteProductLedger::open(&ledger.0).expect("fixture must initialize"));
    let before = artifacts(&ledger.0);

    drop(SqliteProductLedger::open(&ledger.0).expect("canonical v1 must reopen"));

    assert_eq!(artifacts(&ledger.0), before);
}

#[test]
fn migration_key_or_checksum_mismatch_is_rejected_without_changes() {
    // One past the current version, derived rather than written down, so the
    // "extra" row stays extra across schema bumps.
    let extra = format!(
        "INSERT INTO schema_migrations(version,migration_key,checksum) VALUES({},'{:04}_untrusted','sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff')",
        CURRENT_SCHEMA_VERSION + 1,
        CURRENT_SCHEMA_VERSION + 1
    );
    for (label, mutation) in [
        (
            "migration-key",
            "UPDATE schema_migrations SET migration_key='0001_untrusted' WHERE version=1",
        ),
        (
            "migration-checksum",
            "UPDATE schema_migrations SET checksum='sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' WHERE version=2",
        ),
        ("migration-extra", extra.as_str()),
    ] {
        let ledger = SyntheticLedger::new(label);
        drop(SqliteProductLedger::open(&ledger.0).expect("fixture must initialize"));
        let connection = Connection::open(&ledger.0).expect("fixture must open");
        connection.execute(mutation, []).expect("fixture mutation must succeed");
        drop(connection);
        assert_rejected_unchanged(&ledger.0);
    }
}

#[test]
fn prerelease_metadata_only_v1_is_not_silently_adopted() {
    let ledger = SyntheticLedger::new("prerelease-v1");
    let connection = Connection::open(&ledger.0).expect("fixture must open");
    connection
        .execute_batch(
            "PRAGMA application_id=1347240705;
             PRAGMA user_version=1;
             CREATE TABLE ledger_metadata(
                 singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                 schema_version INTEGER NOT NULL CHECK(schema_version>=0),
                 ledger_revision INTEGER NOT NULL CHECK(ledger_revision>=0)
             ) STRICT;
             INSERT INTO ledger_metadata VALUES(1,1,0);",
        )
        .expect("prerelease fixture must initialize");
    drop(connection);
    let before = artifacts(&ledger.0);
    assert!(matches!(
        SqliteProductLedger::open(&ledger.0),
        Err(LedgerOpenError::UnsupportedSchema { found: 1 })
    ));
    assert_eq!(artifacts(&ledger.0), before);
}

#[test]
fn canonical_enum_checks_and_foreign_keys_fail_closed() {
    let ledger = SyntheticLedger::new("constraints");
    drop(SqliteProductLedger::open(&ledger.0).expect("fixture must initialize"));
    let connection = Connection::open(&ledger.0).expect("fixture must open");
    connection
        .execute_batch("PRAGMA foreign_keys=ON")
        .expect("foreign keys must enable");

    assert!(connection
        .execute(
            "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES('portfolio-1','portfolio',0,'secret',0,0)",
            [],
        )
        .is_err());
    connection
        .execute(
            "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES('portfolio-1','portfolio',0,'unclassified',0,0)",
            [],
        )
        .expect("canonical portfolio identity must insert");
    assert!(connection
        .execute(
            "INSERT INTO portfolios(id,name,details,provenance_kind) VALUES('portfolio-1','Synthetic','Public-safe','model_generated')",
            [],
        )
        .is_err());
    connection
        .execute(
            "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES('project-1','project',0,'unclassified',0,0)",
            [],
        )
        .expect("canonical project identity must insert");
    connection
        .execute(
            "INSERT INTO projects(id,name,start_at,end_at,provenance_kind) VALUES('project-1','Synthetic',0,1,'user_entered')",
            [],
        )
        .expect("canonical project must insert");
    connection
        .execute(
            "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES('milestone-1','product',0,'unclassified',0,0)",
            [],
        )
        .expect("wrong-family aggregate fixture must insert");
    assert!(connection
        .execute(
            "INSERT INTO milestones(id,project_id,name,verification_criteria,due_at,provenance_kind) VALUES('milestone-1','project-1','Synthetic','Public-safe',0,'user_entered')",
            [],
        )
        .is_err());
    connection
        .execute(
            "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES('request-1','action_request',0,'unclassified',0,0)",
            [],
        )
        .expect("canonical request identity must insert");
    assert!(connection
        .execute(
            "INSERT INTO action_requests(id,title,details,state,superseded_premise) VALUES('request-1','Synthetic','Public-safe','done',0)",
            [],
        )
        .is_err());
    assert!(connection
        .execute(
            "INSERT INTO action_requests(id,title,details,state,linked_action_id,superseded_premise) VALUES('request-1','Synthetic','Public-safe','open','missing-action',0)",
            [],
        )
        .is_err());

    connection
        .execute(
            "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES('relationship-1','relationship',0,'unclassified',0,0)",
            [],
        )
        .expect("relationship identity must insert");
    connection
        .execute(
            "INSERT INTO relationships(id,kind) VALUES('relationship-1','portfolio_product')",
            [],
        )
        .expect("relationship must insert");
    assert!(connection
        .execute(
            "INSERT INTO relationship_endpoints(relationship_id,ordinal,target_type,target_id,target_version,target_classification) VALUES('relationship-1',0,'product','portfolio-1',0,'unclassified')",
            [],
        )
        .is_err());

    connection
        .execute(
            "INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,expires_at,created_at) VALUES('prepared-actor',1,'close_issue','digest','unclassified','allowed','not_cancellable_after_submit','head_of_products',10,0)",
            [],
        )
        .expect("prepared fixture must insert");
    assert!(connection
        .execute(
            "INSERT INTO approval_receipts(id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at) VALUES('receipt-actor','prepared-actor','policy_authorized_system','digest','idem',1,10)",
            [],
        )
        .is_err());
}

#[test]
fn every_current_prepared_operation_has_a_typed_round_trip_without_a_blob() {
    let ledger = SyntheticLedger::new("prepared-payloads");
    drop(SqliteProductLedger::open(&ledger.0).expect("fixture must initialize"));
    let connection = Connection::open(&ledger.0).expect("fixture must open");
    connection
        .execute_batch("PRAGMA foreign_keys=ON")
        .expect("foreign keys must enable");

    let cases = [
        ("accept_action_request", "primary_id,primary_version,created_id,created_classification,subject,details,intended_owner_id,due_at", "'request-a',1,'action-a','internal','Subject A','Details A','owner-a',10", "request-a|1|action-a|internal|Subject A|Details A|owner-a|10"),
        ("resolve_decision_request", "primary_id,primary_version,created_id,created_classification,statement,rationale,impact,decision_owner_id,decided_at", "'request-d',2,'decision-d','confidential','Statement D','Rationale D','Impact D','owner-d',20", "request-d|2|decision-d|confidential|Statement D|Rationale D|Impact D|owner-d|20"),
        ("complete_action", "primary_id,primary_version", "'action-c',3", "action-c|3"),
        ("cancel_action", "primary_id,primary_version,rationale", "'action-x',4,'Cancel reason'", "action-x|4|Cancel reason"),
        ("reopen_action", "primary_id,primary_version,reopen_mode,rationale", "'action-r',5,'reopen_completed','Reopen reason'", "action-r|5|reopen_completed|Reopen reason"),
        ("supersede_decision", "primary_id,primary_version,replacement_id,replacement_version,replacement_classification,replacement_statement,replacement_rationale,replacement_impact,replacement_owner_id,replacement_decided_at", "'decision-old',6,'decision-new',0,'restricted','New statement','New rationale','New impact','owner-new',30", "decision-old|6|decision-new|0|restricted|New statement|New rationale|New impact|owner-new|30"),
        ("record_risk_occurrence", "primary_id,primary_version,created_id,created_classification", "'risk-o',7,'issue-o','internal'", "risk-o|7|issue-o|internal"),
        ("close_risk", "primary_id,primary_version,rationale", "'risk-c',8,'Close rationale'", "risk-c|8|Close rationale"),
        ("resolve_issue", "primary_id,primary_version,resolution_type,rationale", "'issue-r',9,'workaround','Resolution rationale'", "issue-r|9|workaround|Resolution rationale"),
        ("close_issue", "primary_id,primary_version", "'issue-c',10", "issue-c|10"),
        ("reopen_issue", "primary_id,primary_version,rationale", "'issue-o',11,'Failed verification'", "issue-o|11|Failed verification"),
    ];
    for (ordinal, (kind, columns, values, expected)) in cases.into_iter().enumerate() {
        let prepared_id = format!("prepared-{ordinal}");
        connection
            .execute(
                "INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,expires_at,created_at) VALUES(?1,1,?2,'digest','unclassified','allowed','not_cancellable_after_submit','head_of_products',100,0)",
                (&prepared_id, kind),
            )
            .expect("typed prepared intent must insert");
        connection
            .execute(
                &format!("INSERT INTO prepared_work_management_payloads(prepared_intent_id,{columns}) VALUES(?1,{values})"),
                [&prepared_id],
            )
            .expect("typed operation payload must insert");
        let expression = columns
            .split(',')
            .map(|column| format!("CAST({column} AS TEXT)"))
            .collect::<Vec<_>>()
            .join("||'|'||");
        let actual: String = connection
            .query_row(
                &format!("SELECT {expression} FROM prepared_work_management_payloads WHERE prepared_intent_id=?1"),
                [&prepared_id],
                |row| row.get(0),
            )
            .expect("typed payload must round trip");
        assert_eq!(actual, expected, "round-trip mismatch for {kind}");
    }

    connection
        .execute(
            "INSERT INTO prepared_resulting_action_requests VALUES('prepared-1',0,'resulting-1','Subject','Details','owner',40,'internal')",
            [],
        )
        .expect("resulting Action Request fields must normalize");
    connection
        .execute(
            "INSERT INTO prepared_evidence_classifications VALUES('prepared-3',0,'evidence-1','confidential')",
            [],
        )
        .expect("Evidence classification fields must normalize");
    connection
        .execute(
            "INSERT INTO prepared_incomplete_downstream VALUES('prepared-5',0,'action','action-downstream',12,'restricted')",
            [],
        )
        .expect("downstream fields must normalize");
    let normalized_children: i64 = connection
        .query_row(
            "SELECT (SELECT count(*) FROM prepared_resulting_action_requests)+(SELECT count(*) FROM prepared_evidence_classifications)+(SELECT count(*) FROM prepared_incomplete_downstream)",
            [],
            |row| row.get(0),
        )
        .expect("normalized child payloads must query");
    assert_eq!(normalized_children, 3);

    let payload_columns = connection
        .prepare(
            "SELECT name FROM pragma_table_xinfo('prepared_work_management_payloads') ORDER BY cid",
        )
        .expect("payload inventory must prepare")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("payload inventory must query")
        .collect::<Result<Vec<_>, _>>()
        .expect("payload inventory must decode");
    assert_eq!(
        payload_columns,
        [
            "prepared_intent_id",
            "primary_id",
            "primary_version",
            "created_id",
            "created_classification",
            "subject",
            "details",
            "intended_owner_id",
            "due_at",
            "statement",
            "rationale",
            "impact",
            "decision_owner_id",
            "decided_at",
            "reopen_mode",
            "resolution_type",
            "replacement_id",
            "replacement_version",
            "replacement_classification",
            "replacement_statement",
            "replacement_rationale",
            "replacement_impact",
            "replacement_owner_id",
            "replacement_decided_at",
        ]
    );
    let blob_columns: i64 = connection
        .query_row(
            "SELECT count(*) FROM pragma_table_xinfo('prepared_work_management_payloads') WHERE upper(type) IN ('BLOB','JSON')",
            [],
            |row| row.get(0),
        )
        .expect("payload types must be inspectable");
    assert_eq!(blob_columns, 0);

    let transition_reason_columns: i64 = connection
        .query_row(
            "SELECT count(*) FROM pragma_table_xinfo('action_transitions') WHERE name='reason' AND type='TEXT'",
            [],
            |row| row.get(0),
        )
        .expect("transition reason field must be inspectable");
    assert_eq!(transition_reason_columns, 1);
}

#[test]
fn h2b_removal_preview_round_trips_confirmation_recovery_and_endpoint_parent() {
    let ledger = SyntheticLedger::new("removal-payload");
    drop(SqliteProductLedger::open(&ledger.0).expect("fixture must initialize"));
    let connection = Connection::open(&ledger.0).expect("fixture must open");
    connection.execute_batch("PRAGMA foreign_keys=ON;
        INSERT INTO aggregate_registry VALUES('relationship-r','relationship',0,'restricted',0,0);
        INSERT INTO relationships(id,kind,purpose) VALUES('relationship-r','project_product',NULL);
        INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,confirmation_challenge,expires_at,created_at) VALUES('prepared-r',1,'relationship.remove','digest','restricted','allowed','not_cancellable_after_submit','head_of_products','REMOVE relationship-r',100,0);
        INSERT INTO prepared_removal_payloads VALUES('prepared-r','relationship-r',0,'project_product','none');
        INSERT INTO prepared_removal_endpoints VALUES('prepared-r',0,'milestone','milestone-r',2,'restricted','project-r');
        INSERT INTO prepared_intent_recovery_evidence VALUES('prepared-r','recovery-r','Synthetic recovery',50,'relationship-r',1);
        INSERT INTO prepared_intent_effects(prepared_intent_id,ordinal,effect_code) VALUES('prepared-r',0,'remove_relationship_record'),('prepared-r',1,'remove_semantic_relationship_index'),('prepared-r',2,'create_idempotency_tombstone');")
        .expect("typed H2b preview must normalize");
    let actual: String = connection.query_row(
        "SELECT p.confirmation_challenge||'|'||r.relationship_id||'|'||r.relationship_version||'|'||r.relationship_kind||'|'||r.purpose||'|'||e.endpoint_type||'|'||e.endpoint_id||'|'||e.endpoint_version||'|'||e.classification||'|'||e.parent_project_id||'|'||x.recovery_evidence_id||'|'||x.name||'|'||x.verified_at||'|'||x.compatible FROM prepared_intents p JOIN prepared_removal_payloads r ON r.prepared_intent_id=p.id JOIN prepared_removal_endpoints e ON e.prepared_intent_id=p.id JOIN prepared_intent_recovery_evidence x ON x.prepared_intent_id=p.id WHERE p.id='prepared-r'",
        [], |row| row.get(0)).expect("H2b payload must round trip");
    assert_eq!(actual, "REMOVE relationship-r|relationship-r|0|project_product|none|milestone|milestone-r|2|restricted|project-r|recovery-r|Synthetic recovery|50|1");
}

#[test]
fn delivery_stored_outcome_round_trips_ordered_audits_and_typed_milestone_mutations() {
    let ledger = SyntheticLedger::new("delivery-stored-outcome");
    drop(SqliteProductLedger::open(&ledger.0).expect("fixture must initialize"));
    let connection = Connection::open(&ledger.0).expect("fixture must open");
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON;
             INSERT INTO aggregate_registry VALUES('project-d','project',1,'restricted',1,2);
             INSERT INTO projects(id,name,start_at,end_at,provenance_kind) VALUES('project-d','Synthetic project',1,2,'synthetic_fixture');
             INSERT INTO aggregate_registry VALUES('milestone-d','milestone',2,'restricted',1,3);
             INSERT INTO milestones(id,project_id,name,verification_criteria,due_at,provenance_kind) VALUES('milestone-d','project-d','Synthetic milestone','Synthetic evidence',3,'synthetic_fixture');
             INSERT INTO audit_events VALUES('audit-primary',2,'head_of_products','portfolio','delivery.project.updated','project','project-d','correlation-original','not_required','not_required','succeeded','complete');
             INSERT INTO audit_events VALUES('audit-derived',3,'policy_authorized_system','classification','delivery.milestone.classification_inherited','milestone','milestone-d','correlation-original','not_required','not_required','succeeded','complete');
             INSERT INTO idempotency_outcomes VALUES('delivery','update_project','idempotency-d','payload-digest','succeeded','project-d',3);
             INSERT INTO idempotency_outcomes VALUES('delivery','create_project','other-id','other-digest','succeeded','project-d',4);
             BEGIN IMMEDIATE;
             INSERT INTO delivery_idempotency_outcomes VALUES('delivery','update_project','idempotency-d','update_project','correlation-original',0);
             INSERT INTO delivery_command_results(namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_start_at,command_end_at,command_classification,command_provenance_kind,result_kind,result_id,result_name,result_start_at,result_end_at,result_classification,result_provenance_kind,result_version,result_created_at,result_updated_at) VALUES('delivery','update_project','idempotency-d','update_project','project-d',1,'Synthetic project',1,2,'restricted','user_entered','project','project-d','Synthetic project',1,2,'restricted','user_entered',2,1,2);
             INSERT INTO delivery_idempotency_outcome_audits VALUES('delivery','update_project','idempotency-d',0,'audit-primary');
             INSERT INTO delivery_idempotency_outcome_audits VALUES('delivery','update_project','idempotency-d',1,'audit-derived');
             INSERT INTO delivery_derived_milestone_mutations VALUES('delivery','update_project','idempotency-d',0,'milestone-d',1,2,'internal','restricted',2,3,'audit-derived');
             COMMIT;",
        )
        .expect("normalized Delivery StoredOutcome must insert");

    let outcome: (String, i64, String) = connection
        .query_row(
            "SELECT correlation_id,operation_ordinal,
                    (SELECT group_concat(audit_event_id,'|') FROM
                        (SELECT audit_event_id FROM delivery_idempotency_outcome_audits a
                         WHERE a.namespace=o.namespace AND a.operation=o.operation AND a.idempotency_id=o.idempotency_id
                         ORDER BY ordinal))
             FROM delivery_idempotency_outcomes o
             WHERE namespace='delivery' AND operation='update_project' AND idempotency_id='idempotency-d'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("ordered audit ids must round trip");
    assert_eq!(
        outcome,
        (
            "correlation-original".to_owned(),
            0,
            "audit-primary|audit-derived".to_owned()
        )
    );
    let mutation: (String, i64, i64, String, String, i64, i64, String) = connection
        .query_row(
            "SELECT milestone_id,previous_version,resulting_version,previous_classification,resulting_classification,previous_updated_at,resulting_updated_at,audit_event_id
             FROM delivery_derived_milestone_mutations
             WHERE namespace='delivery' AND operation='update_project' AND idempotency_id='idempotency-d' ORDER BY ordinal",
            [],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)),
        )
        .expect("typed mutation must round trip");
    assert_eq!(
        mutation,
        (
            "milestone-d".to_owned(),
            1,
            2,
            "internal".to_owned(),
            "restricted".to_owned(),
            2,
            3,
            "audit-derived".to_owned()
        )
    );

    for rejected in [
        "INSERT INTO delivery_idempotency_outcomes VALUES('delivery','create_project','other-id','create_project','correlation-other',0)",
        "INSERT INTO delivery_idempotency_outcomes VALUES('delivery','create_initiative','idempotency-d','create_initiative','correlation-cross-operation',1)",
        "INSERT INTO delivery_idempotency_outcome_audits VALUES('delivery','update_project','idempotency-d',2,'missing-audit')",
        "INSERT INTO delivery_idempotency_outcome_audits VALUES('delivery','update_project','idempotency-d',3,'audit-primary')",
        "INSERT INTO delivery_derived_milestone_mutations VALUES('delivery','update_project','idempotency-d',1,'milestone-d',2,4,'restricted','internal',3,2,'audit-derived')",
    ] {
        assert!(connection.execute(rejected, []).is_err(), "must reject: {rejected}");
    }
    connection.execute_batch("BEGIN IMMEDIATE; INSERT INTO delivery_idempotency_outcomes VALUES('delivery','create_project','other-id','create_project','correlation-other',1);").expect("deferred pair may stage parent first");
    assert!(connection.execute(
        "INSERT INTO delivery_command_results(namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_start_at,command_end_at,command_provenance_kind,result_kind,result_id,result_name,result_start_at,result_end_at,result_classification,result_provenance_kind,result_version,result_created_at,result_updated_at) VALUES('delivery','create_project','other-id','create_project','project-d',1,'Malformed create',1,2,'user_entered','project','project-d','Malformed create',1,2,'restricted','user_entered',2,1,2)",
        [],
    ).is_err(), "create topology must reject update-only versions");
    connection
        .execute_batch("ROLLBACK")
        .expect("malformed staged pair must roll back");

    let non_typed_columns: i64 = connection
        .query_row(
            "SELECT
                (SELECT count(*) FROM pragma_table_xinfo('delivery_idempotency_outcomes') WHERE upper(type) IN ('BLOB','JSON'))+
                (SELECT count(*) FROM pragma_table_xinfo('delivery_command_results') WHERE upper(type) IN ('BLOB','JSON'))+
                (SELECT count(*) FROM pragma_table_xinfo('delivery_idempotency_outcome_audits') WHERE upper(type) IN ('BLOB','JSON'))+
                (SELECT count(*) FROM pragma_table_xinfo('delivery_derived_milestone_mutations') WHERE upper(type) IN ('BLOB','JSON'))",
            [],
            |row| row.get(0),
        )
        .expect("Delivery StoredOutcome column types must be inspectable");
    assert_eq!(non_typed_columns, 0);
}

#[test]
fn delivery_outcome_and_typed_command_result_commit_only_as_an_exact_pair() {
    let ledger = SyntheticLedger::new("delivery-exact-pair");
    drop(SqliteProductLedger::open(&ledger.0).expect("fixture must initialize"));
    let connection = Connection::open(&ledger.0).expect("fixture must open");
    connection.execute_batch("PRAGMA foreign_keys=ON; BEGIN IMMEDIATE; INSERT INTO idempotency_outcomes VALUES('delivery','create_project','missing-child','digest','succeeded','project-x',1); INSERT INTO delivery_idempotency_outcomes VALUES('delivery','create_project','missing-child','create_project','correlation-x',0);").expect("deferred parent must stage");
    assert!(
        connection.execute_batch("COMMIT").is_err(),
        "outcome without exact command/result child must not commit"
    );
    connection
        .execute_batch("ROLLBACK")
        .expect("failed deferred commit must roll back");

    connection.execute_batch("BEGIN IMMEDIATE; INSERT INTO idempotency_outcomes VALUES('delivery','create_project','mismatch','digest','succeeded','project-x',1); INSERT INTO delivery_idempotency_outcomes VALUES('delivery','create_project','mismatch','create_project','correlation-x',0); INSERT INTO delivery_command_results(namespace,operation,idempotency_id,command_kind,command_target_id,command_expected_version,command_name,command_start_at,command_end_at,command_provenance_kind,result_kind,result_id,result_name,result_start_at,result_end_at,result_classification,result_provenance_kind,result_version,result_created_at,result_updated_at) VALUES('delivery','update_project','mismatch','update_project','project-x',1,'Mismatched',1,2,'user_entered','project','project-x','Mismatched',1,2,'internal','user_entered',2,1,2);").expect("mismatched deferred keys may stage");
    assert!(
        connection.execute_batch("COMMIT").is_err(),
        "mismatched command/result pair must not commit"
    );
    connection
        .execute_batch("ROLLBACK")
        .expect("failed mismatched commit must roll back");
}
