//! `pmc.projection/v1` generator/verifier.

use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{
    ActionId, ActionRequestId, AggregateVersion, DecisionId, KpiId, ProductId, ProjectId, RiskId,
};
use pmc_domain::projection_source::{
    ActionProjectionSource, DecisionProjectionSource, KpiProjectionSource,
    LedgerProjectionSnapshot, ProductProjectionSource, ProjectProjectionSource,
    RiskProjectionSource,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{ActionState, DecisionState, RiskState};
use pmc_knowledge::projections::{
    generate_projection_artifact_set, verify_staged_artifact_set, ProjectionVerificationError,
};

fn product(id: &str, revision: u64) -> ProductProjectionSource {
    ProductProjectionSource {
        id: ProductId::parse(id).unwrap(),
        classification: DataClassification::Internal,
        source_revision: AggregateVersion::new(revision).unwrap(),
    }
}

fn empty_snapshot() -> LedgerProjectionSnapshot {
    LedgerProjectionSnapshot {
        schema_version: 40,
        ledger_revision: 100,
        ledger_as_of_utc: UtcTimestamp::from_unix_millis(1_700_000_000_000),
        products: Vec::new(),
        projects: Vec::new(),
        actions: Vec::new(),
        decisions: Vec::new(),
        risks: Vec::new(),
        kpis: Vec::new(),
    }
}

/// A snapshot with at least one record per type, exercising every field
/// path in the generator (lifecycle, relationship refs, due/observed time).
fn populated_snapshot() -> LedgerProjectionSnapshot {
    let mut snapshot = empty_snapshot();
    snapshot.products = vec![product("product-1", 3)];
    snapshot.projects = vec![ProjectProjectionSource {
        id: ProjectId::parse("project-1").unwrap(),
        classification: DataClassification::Public,
        source_revision: AggregateVersion::new(2).unwrap(),
        start_at: UtcTimestamp::from_unix_millis(10_000),
        end_at: UtcTimestamp::from_unix_millis(20_000),
    }];
    snapshot.actions = vec![ActionProjectionSource {
        id: ActionId::parse("action-1").unwrap(),
        classification: DataClassification::Restricted,
        source_revision: AggregateVersion::new(4).unwrap(),
        state: ActionState::InProgress,
        due_at: UtcTimestamp::from_unix_millis(30_000),
        source_request_id: ActionRequestId::parse("request-1").unwrap(),
        source_decision_id: Some(DecisionId::parse("decision-1").unwrap()),
    }];
    snapshot.decisions = vec![DecisionProjectionSource {
        id: DecisionId::parse("decision-1").unwrap(),
        classification: DataClassification::Internal,
        source_revision: AggregateVersion::new(1).unwrap(),
        state: DecisionState::Effective,
        decided_at: UtcTimestamp::from_unix_millis(40_000),
        supersedes_decision_id: None,
        superseded_by_decision_id: None,
    }];
    snapshot.risks = vec![RiskProjectionSource {
        id: RiskId::parse("risk-1").unwrap(),
        classification: DataClassification::Internal,
        source_revision: AggregateVersion::new(2).unwrap(),
        state: RiskState::Occurred,
        next_review_at: Some(UtcTimestamp::from_unix_millis(50_000)),
    }];
    snapshot.kpis = vec![KpiProjectionSource {
        id: KpiId::parse("kpi-1").unwrap(),
        classification: DataClassification::Public,
        source_revision: AggregateVersion::new(1).unwrap(),
    }];
    snapshot
}

#[test]
fn identical_snapshots_produce_byte_identical_artifact_sets() {
    let first = generate_projection_artifact_set(&populated_snapshot());
    let second = generate_projection_artifact_set(&populated_snapshot());
    assert_eq!(first, second);
    assert_eq!(
        first.aggregate_manifest_digest(),
        second.aggregate_manifest_digest()
    );
}

#[test]
fn shuffled_input_order_produces_the_same_sorted_output() {
    let mut forward = empty_snapshot();
    forward.products = vec![product("product-a", 1), product("product-b", 1)];
    let mut reversed = empty_snapshot();
    reversed.products = vec![product("product-b", 1), product("product-a", 1)];

    let forward_set = generate_projection_artifact_set(&forward);
    let reversed_set = generate_projection_artifact_set(&reversed);

    assert_eq!(forward_set, reversed_set);
    let paths: Vec<&str> = forward_set
        .artifacts
        .iter()
        .map(|artifact| artifact.relative_path.as_str())
        .collect();
    assert!(paths[0].starts_with("Products/product-a--"));
    assert!(paths[1].starts_with("Products/product-b--"));
}

#[test]
fn case_different_ids_do_not_collide_even_on_a_case_insensitive_filesystem() {
    let mut snapshot = empty_snapshot();
    snapshot.products = vec![product("product-x", 1), product("PRODUCT-X", 1)];

    let set = generate_projection_artifact_set(&snapshot);

    let lowercased: Vec<String> = set
        .artifacts
        .iter()
        .map(|artifact| artifact.relative_path.to_lowercase())
        .collect();
    assert_ne!(
        lowercased[0], lowercased[1],
        "case-different IDs must not collapse to the same path even case-insensitively"
    );
}

#[test]
fn a_freshly_generated_set_passes_verification() {
    let set = generate_projection_artifact_set(&populated_snapshot());
    assert_eq!(verify_staged_artifact_set(&set), Ok(()));
}

#[test]
fn a_mutated_content_byte_fails_verification() {
    let mut set = generate_projection_artifact_set(&populated_snapshot());
    let last = set.artifacts[0].content.len() - 1;
    set.artifacts[0].content[last] ^= 0x01;

    assert_eq!(
        verify_staged_artifact_set(&set),
        Err(ProjectionVerificationError::ContentMismatch { index: 0 })
    );
}

#[test]
fn a_mutated_managed_payload_hash_fails_verification() {
    let mut set = generate_projection_artifact_set(&populated_snapshot());
    set.artifacts[0].managed_payload_sha256 = "0".repeat(64);

    assert_eq!(
        verify_staged_artifact_set(&set),
        Err(ProjectionVerificationError::PayloadHashMismatch { index: 0 })
    );
}

#[test]
fn a_mutated_final_file_hash_fails_verification() {
    let mut set = generate_projection_artifact_set(&populated_snapshot());
    set.artifacts[0].final_file_sha256 = "0".repeat(64);

    assert_eq!(
        verify_staged_artifact_set(&set),
        Err(ProjectionVerificationError::FinalHashMismatch { index: 0 })
    );
}

#[test]
fn a_mutated_relative_path_fails_verification() {
    let mut set = generate_projection_artifact_set(&populated_snapshot());
    set.artifacts[0].relative_path = "Products/tampered-path.md".to_string();

    assert_eq!(
        verify_staged_artifact_set(&set),
        Err(ProjectionVerificationError::PathMismatch { index: 0 })
    );
}

#[test]
fn every_artifact_carries_the_required_banner_and_schema_fields() {
    let set = generate_projection_artifact_set(&populated_snapshot());
    for artifact in &set.artifacts {
        let text = String::from_utf8(artifact.content.clone()).unwrap();
        assert!(text.starts_with("---\n"));
        assert!(text.contains("pmc_projection_schema: \"pmc.projection/v1\"\n"));
        assert!(text.contains("pmc_managed: true\n"));
        assert!(text.contains(&format!(
            "pmc_record_id: \"{}\"\n",
            artifact.frontmatter.record_id
        )));
        assert!(text.contains(&format!(
            "pmc_managed_payload_sha256: \"{}\"\n",
            artifact.managed_payload_sha256
        )));
        assert!(text.contains("> [!WARNING] Generated read-only Projection\n"));
        assert!(text.contains("Manual changes are never imported into Product Ledger"));
    }
}

#[test]
fn a_record_with_no_optional_fields_omits_them_entirely() {
    let set = generate_projection_artifact_set(&populated_snapshot());
    let product_artifact = set
        .artifacts
        .iter()
        .find(|artifact| artifact.relative_path.starts_with("Products/"))
        .unwrap();
    let text = String::from_utf8(product_artifact.content.clone()).unwrap();

    assert!(!text.contains("pmc_lifecycle"));
    assert!(!text.contains("pmc_relationship_refs"));
    assert!(!text.contains("pmc_due_or_observed_at_utc"));
    assert!(!text.contains("pmc_attention_labels"));
}

#[test]
fn an_action_carries_its_relationship_refs_sorted_and_its_lifecycle() {
    let set = generate_projection_artifact_set(&populated_snapshot());
    let action_artifact = set
        .artifacts
        .iter()
        .find(|artifact| artifact.relative_path.starts_with("Actions/"))
        .unwrap();
    let text = String::from_utf8(action_artifact.content.clone()).unwrap();

    assert!(text.contains("pmc_lifecycle: \"in_progress\"\n"));
    assert!(text.contains("pmc_relationship_refs:\n"));
    assert!(text.contains("  - \"action-request:request-1\"\n"));
    assert!(text.contains("  - \"decision:decision-1\"\n"));
    assert!(text.contains("pmc_due_or_observed_at_utc: \"1970-01-01T00:00:30.000Z\"\n"));
}

/// The exact key set `pmc.projection/v1` may emit.
///
/// This is the versioned allowlist as a forcing function rather than a
/// convention. `ProjectionFrontmatter`'s fields are public, so a later
/// change can add one and render it without touching the schema literal --
/// and the projection contract requires fields introduced after a version to
/// stay absent until reviewed. Adding a rendered key now fails here, which
/// makes the choice explicit: bump the schema version, or record the review that
/// admitted the key into v1.
const V1_FRONTMATTER_KEYS: &[&str] = &[
    "pmc_projection_schema",
    "pmc_managed",
    "pmc_record_type",
    "pmc_record_id",
    "pmc_source_revision",
    "pmc_ledger_schema_version",
    "pmc_ledger_revision",
    "pmc_ledger_as_of_utc",
    "pmc_classification",
    "pmc_lifecycle",
    "pmc_relationship_refs",
    "pmc_due_or_observed_at_utc",
    "pmc_attention_labels",
    "pmc_managed_payload_sha256",
];

#[test]
fn no_frontmatter_key_outside_the_v1_allowlist_is_ever_emitted() {
    let set = generate_projection_artifact_set(&populated_snapshot());
    assert!(!set.artifacts.is_empty(), "fixture must produce artifacts");
    for artifact in &set.artifacts {
        let text = String::from_utf8(artifact.content.clone()).unwrap();
        let frontmatter = text
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .map(|(block, _)| block.to_owned())
            .unwrap_or_else(|| panic!("artifact must open with a closed frontmatter block"));
        for line in frontmatter.lines() {
            // List items belong to the key that introduced them.
            if line.starts_with("  - ") || line.trim().is_empty() {
                continue;
            }
            let key = line
                .split_once(':')
                .map(|(key, _)| key)
                .unwrap_or_else(|| panic!("frontmatter line is not a key: {line}"));
            assert!(
                V1_FRONTMATTER_KEYS.contains(&key),
                "{key} is rendered but is not in the pmc.projection/v1 allowlist"
            );
        }
    }
}
