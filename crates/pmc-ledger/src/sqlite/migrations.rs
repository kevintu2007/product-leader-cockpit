use rusqlite::Transaction;
use sha2::{Digest, Sha256};

use super::{
    schema::{
        V10_ISSUE_H1_SQL, V11_RISK_H2A_TERMINAL_DENIAL_SQL,
        V12_RISK_H2A_TERMINAL_DENIAL_AUDIT_BINDING_SQL, V13_RISK_H2A_PREPARE_SQL,
        V14_RISK_H2A_EXECUTE_SQL, V15_ISSUE_H2A_PREPARE_SQL, V16_ISSUE_H2A_EXECUTE_SQL,
        V17_PORTFOLIO_KPI_OBSERVATION_INHERITED_CLASSIFICATION_SQL,
        V18_EVIDENCE_REFERENCE_GENERALIZE_SQL, V19_EVIDENCE_LINK_SQL, V1_SCHEMA_SQL,
        V20_EVIDENCE_VERIFICATION_UPDATE_SQL, V21_PORTFOLIO_H2A_LOWER_CLASSIFICATION_SQL,
        V22_PORTFOLIO_SIBLINGS_H2A_LOWER_CLASSIFICATION_SQL,
        V23_ACTION_DECISION_RISK_ISSUE_H2A_LOWER_CLASSIFICATION_SQL,
        V24_RISK_H2A_LOWER_CLASSIFICATION_PREPARE_SQL,
        V25_RISK_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL,
        V26_DECISION_H2A_LOWER_CLASSIFICATION_PREPARE_SQL,
        V27_DECISION_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL,
        V28_ISSUE_H2A_LOWER_CLASSIFICATION_PREPARE_SQL,
        V29_ISSUE_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL, V2_ACTION_SQL,
        V30_ACTION_H2A_LOWER_CLASSIFICATION_PREPARE_SQL,
        V31_ACTION_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL,
        V32_DELIVERY_H2A_LOWER_CLASSIFICATION_PREPARED_INTENT_KINDS_SQL,
        V33_DELIVERY_H2A_LOWER_CLASSIFICATION_PREPARE_SQL,
        V34_DELIVERY_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL,
        V35_DELIVERY_H2A_LOWER_CLASSIFICATION_EXECUTE_RESULT_SNAPSHOT_SQL,
        V36_ACTION_DECISION_TRIGGERED_REPLAY_AUTHORITY_SQL,
        V37_DECISION_H2A_SUPERSEDE_REPLAY_AUTHORITY_SQL,
        V38_ACTION_H2A_COMPLETE_EVIDENCE_SNAPSHOT_SQL, V39_EVIDENCE_RELOCATION_SQL,
        V3_DECISION_H1_SQL, V40_EVIDENCE_SUPERSESSION_SQL, V41_MANAGED_PROJECTION_REBUILD_SQL,
        V42_MANAGED_PROJECTION_H1_AUTO_SQL, V43_EVIDENCE_OBSERVED_UNPINNED_SQL,
        V44_EVIDENCE_FINGERPRINT_PIN_SQL, V45_WORK_MANAGEMENT_REJECTION_AND_EVIDENCE_BINDING_SQL,
        V46_RISK_ISSUE_PREPARED_REJECTION_SQL, V47_RECORD_ENTRY_FOUNDATION_SQL,
        V48_EVIDENCE_FROM_FILE_SQL, V4_DECISION_H2A_RESOLVE_SQL,
        V5_DECISION_REPLAY_GLOBAL_ORDINAL_SQL, V6_DECISION_H2A_EVIDENCE_SNAPSHOT_SQL,
        V7_RISK_H1_SQL, V8_DECISION_H2A_CORRELATION_ANCHOR_SQL, V9_RISK_H2A_SQL,
    },
    LedgerOpenError, APPLICATION_ID, CURRENT_SCHEMA_VERSION,
};

#[derive(Clone, Copy)]
pub(super) struct Migration {
    pub(super) version: u32,
    pub(super) key: &'static str,
    pub(super) checksum: &'static str,
    pub(super) sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        key: "0001_initial_authoritative_ledger",
        checksum: "sha256:fb08ddf57769e73729ac5fcbe2bf7c260a69cea9857109896c1d42eb43e63614",
        sql: V1_SCHEMA_SQL,
    },
    Migration {
        version: 2,
        key: "0002_action_replay_authority",
        checksum: "sha256:f3f21983eef98406ceba2f15cbc5e38baa8dae5ddc86a743be00ce715a1fd6f2",
        sql: V2_ACTION_SQL,
    },
    Migration {
        version: 3,
        key: "0003_decision_h1_replay_authority",
        checksum: "sha256:1729f5d28e0f164a9dbd081b39807c00b06d99e5af3e22c18eb04ea7e9f0b1d1",
        sql: V3_DECISION_H1_SQL,
    },
    Migration {
        version: 4,
        key: "0004_decision_h2a_resolve_replay_authority",
        checksum: "sha256:29df3c1585c6cf6eb5c67b3671fe1c6cd5c9edc052d76b7f88a00890f89f2a90",
        sql: V4_DECISION_H2A_RESOLVE_SQL,
    },
    Migration {
        version: 5,
        key: "0005_decision_replay_global_ordinal_repair",
        checksum: "sha256:5d35fede24563856fd7b032f40ec30dc4cb063b7657c615886cfc040adb6242a",
        sql: V5_DECISION_REPLAY_GLOBAL_ORDINAL_SQL,
    },
    Migration {
        version: 6,
        key: "0006_decision_h2a_evidence_snapshot",
        checksum: "sha256:9d18674f3320655d93d8044e18ad8647e233b55678190d9ff7ee3a552b8bbaf9",
        sql: V6_DECISION_H2A_EVIDENCE_SNAPSHOT_SQL,
    },
    Migration {
        version: 7,
        key: "0007_risk_h1_replay_authority",
        checksum: "sha256:bfb9e344dc4b163f85d369037595e903ade876173847ea4adf607376a61376db",
        sql: V7_RISK_H1_SQL,
    },
    Migration {
        version: 8,
        key: "0008_decision_h2a_correlation_anchor",
        checksum: "sha256:8c108d437aa10f7849e19258ce32097c33e9fec3df520e34929ff0d742e58c3d",
        sql: V8_DECISION_H2A_CORRELATION_ANCHOR_SQL,
    },
    Migration {
        version: 9,
        key: "0009_risk_h2a_replay_authority",
        checksum: "sha256:f8e5e3ac151e1a964d010efbd37ae9660c5de743966fe8213d6acbd42a41be8c",
        sql: V9_RISK_H2A_SQL,
    },
    Migration {
        version: 10,
        key: "0010_issue_h1_replay_authority",
        checksum: "sha256:b3a5f33cc27db4c7f1ac2443629cd6f4e8e175c92fec6743304c8a248951b41f",
        sql: V10_ISSUE_H1_SQL,
    },
    Migration {
        version: 11,
        key: "0011_risk_h2a_terminal_denial_replacement",
        checksum: "sha256:13a07a5d79b04ec1351390c92ea038f2bb972ca9fffc8ffe86f8a4fdc8a4e37b",
        sql: V11_RISK_H2A_TERMINAL_DENIAL_SQL,
    },
    Migration {
        version: 12,
        key: "0012_risk_h2a_terminal_denial_audit_binding",
        checksum: "sha256:5c9bba3be6c3b5f1d8c32983acd742f7525bfccf93f35fd5fac5a705a722b9ec",
        sql: V12_RISK_H2A_TERMINAL_DENIAL_AUDIT_BINDING_SQL,
    },
    Migration {
        version: 13,
        key: "0013_risk_h2a_prepare_replay_authority",
        checksum: "sha256:95b63650665ae9af449f0e4cf137a71be5b93a5434e92326df86f30e6c3d5feb",
        sql: V13_RISK_H2A_PREPARE_SQL,
    },
    Migration {
        version: 14,
        key: "0014_risk_h2a_execute_replay_authority",
        checksum: "sha256:9066baa82f9e5654b211fe97440fc0da7e85008aae389fcd488bcca3653a0647",
        sql: V14_RISK_H2A_EXECUTE_SQL,
    },
    Migration {
        version: 15,
        key: "0015_issue_h2a_prepare_replay_authority",
        checksum: "sha256:5883e7987f005814e8665412731d2d4319e072ad2ce76ff427371f3f00585933",
        sql: V15_ISSUE_H2A_PREPARE_SQL,
    },
    Migration {
        version: 16,
        key: "0016_issue_h2a_execute_replay_authority",
        checksum: "sha256:c3e01c97aea0baa7db3a97fe5793685c9f88e87cdbd247c6ae70f27466d67690",
        sql: V16_ISSUE_H2A_EXECUTE_SQL,
    },
    Migration {
        version: 17,
        key: "0017_portfolio_kpi_observation_inherited_classification_repair",
        checksum: "sha256:347c2841268624d37bf829036870ebdf90466967e99e14bdb5fcf280d9e5c933",
        sql: V17_PORTFOLIO_KPI_OBSERVATION_INHERITED_CLASSIFICATION_SQL,
    },
    Migration {
        version: 18,
        key: "0018_evidence_reference_generalize",
        checksum: "sha256:846a9d1ab843313e18fd22cfa02bd6ae50f02eb417448f3ae4b81a2a110a2885",
        sql: V18_EVIDENCE_REFERENCE_GENERALIZE_SQL,
    },
    Migration {
        version: 19,
        key: "0019_evidence_link_replay_authority",
        checksum: "sha256:99b7c3d4db5708cd1747b5d76db1f1625d4dd8387d14efcd6bdf2ddaab3367d7",
        sql: V19_EVIDENCE_LINK_SQL,
    },
    Migration {
        version: 20,
        key: "0020_evidence_verification_update_replay_authority",
        checksum: "sha256:c918a6ee8293cfe8e859d228d2db23ab622602444a57c95c7d68bfc1a0bdfe5e",
        sql: V20_EVIDENCE_VERIFICATION_UPDATE_SQL,
    },
    Migration {
        version: 21,
        key: "0021_portfolio_h2a_lower_classification_replay_authority",
        checksum: "sha256:a28c4622f509b5ef4354bfb5b7bd9b8d898c84357e5f077e138a7478bc75954e",
        sql: V21_PORTFOLIO_H2A_LOWER_CLASSIFICATION_SQL,
    },
    Migration {
        version: 22,
        key: "0022_portfolio_siblings_h2a_lower_classification_replay_authority",
        checksum: "sha256:47cca153a2113e05b3f376906f5c0bacfdfe7a046ddd940ea6a517fce9c08a47",
        sql: V22_PORTFOLIO_SIBLINGS_H2A_LOWER_CLASSIFICATION_SQL,
    },
    Migration {
        version: 23,
        key: "0023_action_decision_risk_issue_h2a_lower_classification_replay_authority",
        checksum: "sha256:968d33c5f61db97a809acfb01fc2569520585d1638400981526e715acf9508a3",
        sql: V23_ACTION_DECISION_RISK_ISSUE_H2A_LOWER_CLASSIFICATION_SQL,
    },
    Migration {
        version: 24,
        key: "0024_risk_h2a_lower_classification_prepare_replay_authority",
        checksum: "sha256:e605cc70db1ff4fac8e051a5841d10701a945363c921c4de21f49398e2111e6a",
        sql: V24_RISK_H2A_LOWER_CLASSIFICATION_PREPARE_SQL,
    },
    Migration {
        version: 25,
        key: "0025_risk_h2a_lower_classification_execute_replay_authority",
        checksum: "sha256:abd6ef0575c8ac983a4ac107563d77ca6c55fa0ae2d91f2a5b82c4fd85623519",
        sql: V25_RISK_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL,
    },
    Migration {
        version: 26,
        key: "0026_decision_h2a_lower_classification_prepare_replay_authority",
        checksum: "sha256:c1e1bb593f33ef12c4dfffaa5d643414b5ae40aca6dc00ada46162442caf010a",
        sql: V26_DECISION_H2A_LOWER_CLASSIFICATION_PREPARE_SQL,
    },
    Migration {
        version: 27,
        key: "0027_decision_h2a_lower_classification_execute_replay_authority",
        checksum: "sha256:86f7963ef0f337f5a6e84acc7cb093b62e8e5ab54d6c7cd39d011cf740cbb0a3",
        sql: V27_DECISION_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL,
    },
    Migration {
        version: 28,
        key: "0028_issue_h2a_lower_classification_prepare_replay_authority",
        checksum: "sha256:babab2989a631d593f201dc777c319c9cc22c79ef40fb72fe83751cd52712dcb",
        sql: V28_ISSUE_H2A_LOWER_CLASSIFICATION_PREPARE_SQL,
    },
    Migration {
        version: 29,
        key: "0029_issue_h2a_lower_classification_execute_replay_authority",
        checksum: "sha256:15d8617792896903c435b50f4f676a75912e5def65b69353134b682785592dee",
        sql: V29_ISSUE_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL,
    },
    Migration {
        version: 30,
        key: "0030_action_h2a_lower_classification_prepare_replay_authority",
        checksum: "sha256:d9fb6f1d5e988fe363b1aa9fc0e9e3739016f1a83054a7f80f1cfc1698e74138",
        sql: V30_ACTION_H2A_LOWER_CLASSIFICATION_PREPARE_SQL,
    },
    Migration {
        version: 31,
        key: "0031_action_h2a_lower_classification_execute_replay_authority",
        checksum: "sha256:b28be312b064dfecf4643e9ce8a3b939784647560ab8402c3e6979e7df25370f",
        sql: V31_ACTION_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL,
    },
    Migration {
        version: 32,
        key: "0032_delivery_h2a_lower_classification_prepared_intent_kinds",
        checksum: "sha256:459ffdffc141ba4cb31648f1b00ce7aa0e330ddc3129a69930607cd289009c20",
        sql: V32_DELIVERY_H2A_LOWER_CLASSIFICATION_PREPARED_INTENT_KINDS_SQL,
    },
    Migration {
        version: 33,
        key: "0033_delivery_h2a_lower_classification_prepare_replay_authority",
        checksum: "sha256:67b62b4d9a3249d6b0bfcce1157febc5c455129667b5ab8c62581571d9d08538",
        sql: V33_DELIVERY_H2A_LOWER_CLASSIFICATION_PREPARE_SQL,
    },
    Migration {
        version: 34,
        key: "0034_delivery_h2a_lower_classification_execute_replay_authority",
        checksum: "sha256:9cb91a7143affdd794af1317a9f89e832d165a4dc0974eff4e35d740610b0c27",
        sql: V34_DELIVERY_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL,
    },
    Migration {
        version: 35,
        key: "0035_delivery_h2a_lower_classification_execute_result_snapshot",
        checksum: "sha256:35cb22294066c599f5f8ec84ca1611e7cd36361b9f598ee22988146ca5c7d634",
        sql: V35_DELIVERY_H2A_LOWER_CLASSIFICATION_EXECUTE_RESULT_SNAPSHOT_SQL,
    },
    Migration {
        version: 36,
        key: "0036_action_decision_triggered_replay_authority",
        checksum: "sha256:f808296cf5953defbd32ab2d5ff32b019d6ce6f7980907c088a7fa69b4b0d70a",
        sql: V36_ACTION_DECISION_TRIGGERED_REPLAY_AUTHORITY_SQL,
    },
    Migration {
        version: 37,
        key: "0037_decision_h2a_supersede_replay_authority",
        checksum: "sha256:529a576f909ac431139476d32dddef84866a58989a4b5fa06d01ed9d31554b6e",
        sql: V37_DECISION_H2A_SUPERSEDE_REPLAY_AUTHORITY_SQL,
    },
    Migration {
        version: 38,
        key: "0038_action_h2a_complete_evidence_snapshot",
        checksum: "sha256:c806c87b86e502454f6e77c07b2833704cb811d9cb1568835cd8641da18aafcb",
        sql: V38_ACTION_H2A_COMPLETE_EVIDENCE_SNAPSHOT_SQL,
    },
    Migration {
        version: 39,
        key: "0039_evidence_relocation_replay_authority",
        checksum: "sha256:6654135ba5f780fbf9ebfcdbb59997c9e91cb15bfecd2c77d14917d7c3f89794",
        sql: V39_EVIDENCE_RELOCATION_SQL,
    },
    Migration {
        version: 40,
        key: "0040_evidence_supersession_h2a_replay_authority",
        checksum: "sha256:2ce78d3ceb6576f2f3ed9527d98cf143023cab148bf9fea5e73c01971c4f797f",
        sql: V40_EVIDENCE_SUPERSESSION_SQL,
    },
    Migration {
        version: 41,
        key: "0041_managed_projection_rebuild_h2a_persistence",
        checksum: "sha256:1672216c89a08abe89657240561771c941e8ee0bee9d3b849d5ff3a499dd2823",
        sql: V41_MANAGED_PROJECTION_REBUILD_SQL,
    },
    Migration {
        version: 42,
        key: "0042_managed_projection_h1_auto_publication",
        checksum: "sha256:16c7f0bd1a200bca3bf3a6af824df3823844430d212b271c3ead6bf36046ae6b",
        sql: V42_MANAGED_PROJECTION_H1_AUTO_SQL,
    },
    Migration {
        version: 43,
        key: "0043_evidence_observed_unpinned_verification",
        checksum: "sha256:e46ae813773c112fb7a7edbd4e5ea4cdac3bb8fa11f6f18ef85c6d0114a987ea",
        sql: V43_EVIDENCE_OBSERVED_UNPINNED_SQL,
    },
    Migration {
        version: 44,
        key: "0044_evidence_fingerprint_pin_replay_authority",
        checksum: "sha256:6bbcc105a74e21e46e99a8576ad9163b9aa8b470e216e87e42db04cda4ad3541",
        sql: V44_EVIDENCE_FINGERPRINT_PIN_SQL,
    },
    Migration {
        version: 45,
        key: "0045_work_management_rejection_and_evidence_binding",
        checksum: "sha256:627c6d4fb88191651d671e54379797553764d3ec300e90d1069cacaf5c51f008",
        sql: V45_WORK_MANAGEMENT_REJECTION_AND_EVIDENCE_BINDING_SQL,
    },
    Migration {
        version: 46,
        key: "0046_risk_issue_prepared_rejection",
        checksum: "sha256:9eb8562ae45a5d685f7ecc84142d9ee5bcbb113b77cea3d7013406b9d5635c28",
        sql: V46_RISK_ISSUE_PREPARED_REJECTION_SQL,
    },
    Migration {
        version: 47,
        key: "0047_record_entry_foundation",
        checksum: "sha256:ec9f56c823e333aeea1d95f5f592c0eeae4c21914d91cdf2101ac18254163f16",
        sql: V47_RECORD_ENTRY_FOUNDATION_SQL,
    },
    Migration {
        version: 48,
        key: "0048_evidence_from_file",
        checksum: "sha256:837033a7b0772d6fca00fa3cc76490eb13b4013aebf066281172f6e89bcb8793",
        sql: V48_EVIDENCE_FROM_FILE_SQL,
    },
];

pub(super) fn bootstrap(transaction: &Transaction<'_>) -> Result<(), LedgerOpenError> {
    for migration in MIGRATIONS {
        apply_migration(transaction, migration, false)?;
    }
    transaction
        .execute(
            "INSERT INTO ledger_metadata(singleton,schema_version,ledger_revision) VALUES(1,?1,0)",
            [i64::from(CURRENT_SCHEMA_VERSION)],
        )
        .map_err(super::classify_open_error)?;
    transaction
        .execute_batch(&format!(
            "PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version={CURRENT_SCHEMA_VERSION};"
        ))
        .map_err(super::classify_open_error)
}

fn apply_migration(
    transaction: &Transaction<'_>,
    migration: &Migration,
    inject_failure: bool,
) -> Result<(), LedgerOpenError> {
    validate_descriptor(migration)?;
    transaction
        .execute_batch(migration.sql)
        .map_err(super::classify_open_error)?;
    if inject_failure {
        return Err(LedgerOpenError::StorageUnavailable);
    }
    transaction
        .execute(
            "INSERT INTO schema_migrations(version,migration_key,checksum) VALUES(?1,?2,?3)",
            (
                i64::from(migration.version),
                migration.key,
                migration.checksum,
            ),
        )
        .map_err(super::classify_open_error)?;
    Ok(())
}

#[cfg(test)]
fn migrate_v1_to_current_for_test(
    transaction: &Transaction<'_>,
    inject_failure_after_all_changes: bool,
) -> Result<(), LedgerOpenError> {
    apply_migration(transaction, &MIGRATIONS[1], false)?;
    apply_migration(transaction, &MIGRATIONS[2], false)?;
    apply_migration(transaction, &MIGRATIONS[3], false)?;
    apply_migration(transaction, &MIGRATIONS[4], false)?;
    apply_migration(transaction, &MIGRATIONS[5], false)?;
    apply_migration(transaction, &MIGRATIONS[6], false)?;
    apply_migration(transaction, &MIGRATIONS[7], false)?;
    apply_migration(transaction, &MIGRATIONS[8], false)?;
    apply_migration(transaction, &MIGRATIONS[9], false)?;
    apply_migration(transaction, &MIGRATIONS[10], false)?;
    apply_migration(transaction, &MIGRATIONS[11], false)?;
    apply_migration(transaction, &MIGRATIONS[12], false)?;
    apply_migration(transaction, &MIGRATIONS[13], false)?;
    apply_migration(transaction, &MIGRATIONS[14], false)?;
    apply_migration(transaction, &MIGRATIONS[15], false)?;
    apply_migration(transaction, &MIGRATIONS[16], false)?;
    apply_migration(transaction, &MIGRATIONS[17], false)?;
    apply_migration(transaction, &MIGRATIONS[18], false)?;
    apply_migration(transaction, &MIGRATIONS[19], false)?;
    apply_migration(transaction, &MIGRATIONS[20], false)?;
    apply_migration(transaction, &MIGRATIONS[21], false)?;
    apply_migration(transaction, &MIGRATIONS[22], false)?;
    apply_migration(transaction, &MIGRATIONS[23], false)?;
    apply_migration(transaction, &MIGRATIONS[24], false)?;
    apply_migration(transaction, &MIGRATIONS[25], false)?;
    apply_migration(transaction, &MIGRATIONS[26], false)?;
    apply_migration(transaction, &MIGRATIONS[27], false)?;
    apply_migration(transaction, &MIGRATIONS[28], false)?;
    apply_migration(transaction, &MIGRATIONS[29], false)?;
    apply_migration(transaction, &MIGRATIONS[30], false)?;
    apply_migration(transaction, &MIGRATIONS[31], false)?;
    apply_migration(transaction, &MIGRATIONS[32], false)?;
    apply_migration(transaction, &MIGRATIONS[33], false)?;
    apply_migration(transaction, &MIGRATIONS[34], false)?;
    apply_migration(transaction, &MIGRATIONS[35], false)?;
    apply_migration(transaction, &MIGRATIONS[36], false)?;
    apply_migration(transaction, &MIGRATIONS[37], false)?;
    apply_migration(transaction, &MIGRATIONS[38], false)?;
    apply_migration(transaction, &MIGRATIONS[39], false)?;
    apply_migration(transaction, &MIGRATIONS[40], false)?;
    apply_migration(transaction, &MIGRATIONS[41], false)?;
    apply_migration(transaction, &MIGRATIONS[42], false)?;
    apply_migration(transaction, &MIGRATIONS[43], false)?;
    apply_migration(transaction, &MIGRATIONS[44], false)?;
    apply_migration(transaction, &MIGRATIONS[45], false)?;
    apply_migration(transaction, &MIGRATIONS[46], false)?;
    apply_migration(transaction, &MIGRATIONS[47], false)?;
    transaction
        .execute(
            "UPDATE ledger_metadata SET schema_version=?1 WHERE singleton=1",
            [i64::from(CURRENT_SCHEMA_VERSION)],
        )
        .map_err(super::classify_open_error)?;
    transaction
        .execute_batch(&format!("PRAGMA user_version={CURRENT_SCHEMA_VERSION};"))
        .map_err(super::classify_open_error)?;
    if inject_failure_after_all_changes {
        return Err(LedgerOpenError::StorageUnavailable);
    }
    Ok(())
}

/// The production descriptors, for tests that build their own next version.
#[cfg(test)]
pub(super) const MIGRATIONS_FOR_TESTS: &[Migration] = MIGRATIONS;

/// A fresh, empty Ledger file at an older version this binary can upgrade
/// from — what a previous release's `open` would have written — so upgrade
/// and inspection tests have a real source without a checked-in file.
#[cfg(test)]
#[allow(clippy::expect_used)]
pub(super) fn bootstrap_at_for_test(path: &std::path::Path, version: u32) {
    let mut connection = rusqlite::Connection::open(path).expect("synthetic database");
    let transaction = connection.transaction().expect("synthetic transaction");
    let descriptors =
        descriptors_through(MIGRATIONS, version).expect("a version this binary ships");
    for migration in descriptors {
        apply_migration(&transaction, migration, false).expect("descriptor must apply");
    }
    transaction
        .execute(
            "INSERT INTO ledger_metadata(singleton,schema_version,ledger_revision) VALUES(1,?1,0)",
            [i64::from(version)],
        )
        .expect("metadata must be recorded");
    transaction
        .execute_batch(&format!(
            "PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version={version};"
        ))
        .expect("identity must be recorded");
    transaction.commit().expect("bootstrap must commit");
}
/// The descriptors this binary ships: versions 1 through
/// `CURRENT_SCHEMA_VERSION`, contiguous.
pub(super) fn production_descriptors() -> &'static [Migration] {
    MIGRATIONS
}

/// The descriptors that bring a Ledger to `version`, if this binary has
/// every one of them: `descriptors[..version]`, checked contiguous.
pub(super) fn descriptors_through(
    descriptors: &'static [Migration],
    version: u32,
) -> Option<&'static [Migration]> {
    let prefix = descriptors.get(..usize::try_from(version).ok()?)?;
    prefix
        .iter()
        .zip(1_u32..)
        .all(|(migration, expected)| migration.version == expected)
        .then_some(prefix)
}

pub(super) fn validate_registry(connection: &rusqlite::Connection) -> Result<(), LedgerOpenError> {
    validate_registry_against(connection, MIGRATIONS)
}

/// The recorded migrations are exactly these descriptors — no row missing,
/// changed, repeated or extra.
pub(super) fn validate_registry_against(
    connection: &rusqlite::Connection,
    descriptors: &[Migration],
) -> Result<(), LedgerOpenError> {
    validate_compiled_descriptors()?;
    let mut statement = connection
        .prepare("SELECT version,migration_key,checksum FROM schema_migrations ORDER BY version")
        .map_err(|_| LedgerOpenError::InvalidMetadata)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|_| LedgerOpenError::InvalidMetadata)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| LedgerOpenError::InvalidMetadata)?;
    let expected = descriptors
        .iter()
        .map(|migration| {
            (
                i64::from(migration.version),
                migration.key.to_owned(),
                migration.checksum.to_owned(),
            )
        })
        .collect::<Vec<_>>();
    if rows != expected {
        return Err(LedgerOpenError::InvalidMetadata);
    }
    Ok(())
}

/// The descriptors are compiled into this binary and cannot change while it runs, so their
/// checksums are verified once per process rather than on every transaction.
fn validate_compiled_descriptors() -> Result<(), LedgerOpenError> {
    static VALID: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let valid = *VALID.get_or_init(|| {
        MIGRATIONS
            .iter()
            .all(|migration| validate_descriptor(migration).is_ok())
    });
    if valid {
        Ok(())
    } else {
        Err(LedgerOpenError::InvalidMetadata)
    }
}

pub(super) fn validate_descriptor(migration: &Migration) -> Result<(), LedgerOpenError> {
    let computed = format!("sha256:{:x}", Sha256::digest(migration.sql.as_bytes()));
    if computed != migration.checksum {
        return Err(LedgerOpenError::InvalidMetadata);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::schema::schema_objects;
    use rusqlite::Connection;

    #[test]
    fn migration_descriptor_rejects_a_checksum_for_different_source() {
        let altered = Migration {
            version: 1,
            key: "0001_initial_authoritative_ledger",
            checksum: MIGRATIONS[0].checksum,
            sql: "CREATE TABLE altered(id TEXT PRIMARY KEY) STRICT;",
        };
        assert_eq!(
            validate_descriptor(&altered),
            Err(LedgerOpenError::InvalidMetadata)
        );
    }

    fn committed_v1_fixture() -> Connection {
        let mut connection =
            Connection::open_in_memory().unwrap_or_else(|_| panic!("synthetic database"));
        let transaction = connection
            .transaction()
            .unwrap_or_else(|_| panic!("synthetic transaction"));
        transaction
            .execute_batch(V1_SCHEMA_SQL)
            .unwrap_or_else(|_| panic!("v1 fixture must bootstrap"));
        transaction
            .execute_batch(&format!(
                "PRAGMA application_id={APPLICATION_ID}; PRAGMA user_version=1;"
            ))
            .unwrap_or_else(|_| panic!("v1 identity must be recorded"));
        transaction
            .execute(
                "INSERT INTO schema_migrations(version,migration_key,checksum) VALUES(1,?1,?2)",
                (MIGRATIONS[0].key, MIGRATIONS[0].checksum),
            )
            .unwrap_or_else(|_| panic!("v1 registry must be recorded"));
        transaction
            .execute(
                "INSERT INTO ledger_metadata(singleton,schema_version,ledger_revision) VALUES(1,1,7)",
                [],
            )
            .unwrap_or_else(|_| panic!("v1 metadata must be recorded"));
        transaction
            .execute(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES('sentinel-owner','stakeholder',0,'internal',0,0)",
                [],
            )
            .unwrap_or_else(|_| panic!("v1 sentinel registry must be recorded"));
        transaction
            .execute(
                "INSERT INTO stakeholders(id, name, kind, provenance_kind, provenance_reference) VALUES('sentinel-owner','Synthetic owner','person','synthetic_fixture','migration-test')",
                [],
            )
            .unwrap_or_else(|_| panic!("v1 sentinel data must be recorded"));
        transaction
            .commit()
            .unwrap_or_else(|_| panic!("v1 fixture must be committed"));
        connection
    }

    fn committed_v7_decision_h2a_fixture() -> Connection {
        let mut connection = committed_v1_fixture();
        let transaction = connection
            .transaction()
            .unwrap_or_else(|_| panic!("synthetic v7 transaction"));
        for migration in &MIGRATIONS[1..7] {
            apply_migration(&transaction, migration, false)
                .unwrap_or_else(|_| panic!("v7 fixture migration must succeed"));
        }
        transaction
            .execute(
                "UPDATE ledger_metadata SET schema_version=7 WHERE singleton=1",
                [],
            )
            .unwrap_or_else(|_| panic!("v7 fixture metadata must be recorded"));
        transaction
            .execute_batch("PRAGMA user_version=7;")
            .unwrap_or_else(|_| panic!("v7 fixture identity must be recorded"));
        transaction
            .execute_batch(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES('synthetic-v7-request','decision_request',2,'internal',100,150),('synthetic-v7-decision','decision',1,'internal',150,150);
                 INSERT INTO decision_requests(id,subject,details,intended_owner_id,state,withdrawal_rationale,linked_decision_id) VALUES('synthetic-v7-request','Synthetic decision request','Synthetic only; no organizational data.','sentinel-owner','resolved',NULL,'synthetic-v7-decision');
                 INSERT INTO support_witnesses(id,requirement,disposition,classification) VALUES('synthetic-v7-support','evidence_or_judgment','judgment_satisfied','internal');
                 INSERT INTO decisions(id,source_request_id,statement,rationale,impact,owner_id,decided_at,state,support_id,supersedes_decision_id,superseded_by_decision_id) VALUES('synthetic-v7-decision','synthetic-v7-request','Synthetic statement','Synthetic rationale','Synthetic impact','sentinel-owner',150,'effective','synthetic-v7-support',NULL,NULL);
                 INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at,consumed_at) VALUES('synthetic-v7-prepared',1,'resolve_decision_request','synthetic-v7-digest','internal','allowed','not_cancellable_after_submit','head_of_products',NULL,NULL,200,100,150);
                 INSERT INTO approval_receipts(id,prepared_intent_id,actor,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at) VALUES('synthetic-v7-receipt','synthetic-v7-prepared','head_of_products','synthetic-v7-digest','synthetic-v7-execute',150,200,150);
                 INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES('synthetic-v7-audit-prepare',100,'head_of_products','work_management','decision_request.resolve_prepared','decision_request','synthetic-v7-request','synthetic-v7-correlation-prepare','allowed','not_required','succeeded','complete'),('synthetic-v7-audit-execute-resolved',150,'head_of_products','work_management','decision_request.resolved','decision_request','synthetic-v7-request','synthetic-v7-correlation-execute','allowed','approved','succeeded','complete'),('synthetic-v7-audit-execute-created',150,'head_of_products','work_management','decision.created','decision','synthetic-v7-decision','synthetic-v7-correlation-execute','allowed','approved','succeeded','complete'),('synthetic-v7-audit-execute-linked',150,'head_of_products','work_management','decision_request.decision_linked','decision_request','synthetic-v7-request','synthetic-v7-correlation-execute','allowed','approved','succeeded','complete');
                 INSERT INTO audit_effects(audit_event_id,ordinal,effect_code,scope,target_type,target_id) VALUES('synthetic-v7-audit-execute-resolved',0,'decision_request.resolved','complete','decision_request','synthetic-v7-request'),('synthetic-v7-audit-execute-created',0,'decision.created','complete','decision','synthetic-v7-decision'),('synthetic-v7-audit-execute-linked',0,'decision_request.decision_linked','complete','decision_request','synthetic-v7-request');
                 INSERT INTO decision_h2a_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference) VALUES('synthetic-v7-prepare','prepare_resolve','synthetic-v7-correlation-prepare',0,'prepared','synthetic-v7-prepared');
                 INSERT INTO decision_h2a_command_prepare_resolves(idempotency_id,request_id,expected_version,statement,rationale,impact) VALUES('synthetic-v7-prepare','synthetic-v7-request',1,'Synthetic statement','Synthetic rationale','Synthetic impact');
                 INSERT INTO decision_h2a_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES('synthetic-v7-prepare',0,'synthetic-v7-audit-prepare','synthetic-v7-correlation-prepare');
                 INSERT INTO decision_h2a_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_kind,result_reference,prepared_intent_id,approval_receipt_id) VALUES('synthetic-v7-execute','execute_resolve','synthetic-v7-correlation-execute',1,'resolved','synthetic-v7-decision','synthetic-v7-prepared','synthetic-v7-receipt');
                 INSERT INTO decision_h2a_command_execute_resolves(idempotency_id,prepared_id,actor,acknowledged_digest) VALUES('synthetic-v7-execute','synthetic-v7-prepared','head_of_products','synthetic-v7-digest');
                 INSERT INTO decision_h2a_replay_audits(idempotency_id,ordinal,audit_event_id,correlation_id) VALUES('synthetic-v7-execute',0,'synthetic-v7-audit-execute-resolved','synthetic-v7-correlation-execute'),('synthetic-v7-execute',1,'synthetic-v7-audit-execute-created','synthetic-v7-correlation-execute'),('synthetic-v7-execute',2,'synthetic-v7-audit-execute-linked','synthetic-v7-correlation-execute');",
            )
            .unwrap_or_else(|error| {
                panic!("populated v7 decision H2a fixture must be recorded: {error}")
            });
        transaction
            .commit()
            .unwrap_or_else(|_| panic!("populated v7 fixture must commit"));
        connection
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn v7_to_v8_backfills_every_populated_decision_h2a_correlation_anchor() {
        let mut connection = committed_v7_decision_h2a_fixture();
        let transaction = connection
            .transaction()
            .unwrap_or_else(|_| panic!("synthetic v8 transaction"));
        apply_migration(&transaction, &MIGRATIONS[7], false)
            .unwrap_or_else(|_| panic!("v8 migration must succeed"));
        transaction
            .execute(
                "UPDATE ledger_metadata SET schema_version=8 WHERE singleton=1",
                [],
            )
            .unwrap_or_else(|_| panic!("v8 fixture metadata must be recorded"));
        transaction
            .execute_batch("PRAGMA user_version=8;")
            .unwrap_or_else(|_| panic!("v8 fixture identity must be recorded"));
        transaction
            .commit()
            .unwrap_or_else(|_| panic!("v8 fixture must commit"));

        let anchors: Vec<(String, String)> = connection
            .prepare("SELECT idempotency_id,correlation_id FROM decision_h2a_correlation_anchors ORDER BY idempotency_id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            anchors,
            vec![
                (
                    "synthetic-v7-execute".to_owned(),
                    "synthetic-v7-correlation-execute".to_owned()
                ),
                (
                    "synthetic-v7-prepare".to_owned(),
                    "synthetic-v7-correlation-prepare".to_owned()
                ),
            ]
        );
        let execute_audits: Vec<(i64, String, String, String, String, String, String)> = connection
            .prepare("SELECT replay.ordinal,audit.id,audit.event_code,audit.target_type,audit.target_id,audit.correlation_id,effect.effect_code FROM decision_h2a_replay_audits replay JOIN audit_events audit ON audit.id=replay.audit_event_id JOIN audit_effects effect ON effect.audit_event_id=audit.id AND effect.ordinal=0 WHERE replay.idempotency_id='synthetic-v7-execute' ORDER BY replay.ordinal")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            execute_audits,
            vec![
                (
                    0,
                    "synthetic-v7-audit-execute-resolved".to_owned(),
                    "decision_request.resolved".to_owned(),
                    "decision_request".to_owned(),
                    "synthetic-v7-request".to_owned(),
                    "synthetic-v7-correlation-execute".to_owned(),
                    "decision_request.resolved".to_owned()
                ),
                (
                    1,
                    "synthetic-v7-audit-execute-created".to_owned(),
                    "decision.created".to_owned(),
                    "decision".to_owned(),
                    "synthetic-v7-decision".to_owned(),
                    "synthetic-v7-correlation-execute".to_owned(),
                    "decision.created".to_owned()
                ),
                (
                    2,
                    "synthetic-v7-audit-execute-linked".to_owned(),
                    "decision_request.decision_linked".to_owned(),
                    "decision_request".to_owned(),
                    "synthetic-v7-request".to_owned(),
                    "synthetic-v7-correlation-execute".to_owned(),
                    "decision_request.decision_linked".to_owned()
                ),
            ]
        );
        let receipt: (String, String, String, String, i64, i64, i64) = connection.query_row("SELECT id,prepared_intent_id,acknowledged_payload_digest,idempotency_id,approved_at,expires_at,consumed_at FROM approval_receipts WHERE id='synthetic-v7-receipt'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?))).unwrap();
        assert_eq!(
            receipt,
            (
                "synthetic-v7-receipt".to_owned(),
                "synthetic-v7-prepared".to_owned(),
                "synthetic-v7-digest".to_owned(),
                "synthetic-v7-execute".to_owned(),
                150,
                200,
                150
            )
        );
        assert!(connection.execute("UPDATE decision_h2a_replay_operations SET correlation_id='synthetic-tamper' WHERE idempotency_id='synthetic-v7-execute'", []).is_err());
        assert_eq!(connection.query_row("SELECT correlation_id FROM decision_h2a_replay_operations WHERE idempotency_id='synthetic-v7-execute'", [], |row| row.get::<_, String>(0)).unwrap(), "synthetic-v7-correlation-execute");
    }

    fn committed_v43_evidence_fixture() -> Connection {
        let mut connection = committed_v1_fixture();
        let transaction = connection
            .transaction()
            .unwrap_or_else(|_| panic!("synthetic v43 transaction"));
        for migration in &MIGRATIONS[1..43] {
            apply_migration(&transaction, migration, false)
                .unwrap_or_else(|_| panic!("v43 fixture migration must succeed"));
        }
        transaction
            .execute(
                "UPDATE ledger_metadata SET schema_version=43 WHERE singleton=1",
                [],
            )
            .unwrap_or_else(|_| panic!("v43 fixture metadata must be recorded"));
        transaction
            .execute_batch("PRAGMA user_version=43;")
            .unwrap_or_else(|_| panic!("v43 fixture identity must be recorded"));
        transaction
            .execute_batch(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES('synthetic-v43-evidence','evidence_reference',2,'internal',100,150);
                 INSERT INTO evidence_references(id,role,verification,integrity_digest,last_verified_at,vault_relative_path,fingerprint_algorithm,fingerprint_digest,provenance_kind,provenance_reference) VALUES('synthetic-v43-evidence',NULL,'verified','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',150,'evidence/synthetic-v43.md','sha256','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','synthetic_fixture','migration-test');
                 INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES('synthetic-v43-audit',150,'head_of_products','work_management','evidence.reference_relocated','evidence_reference','synthetic-v43-evidence','synthetic-v43-correlation','allowed','not_required','succeeded','complete');
                 INSERT INTO evidence_relocation_command_results(operation,idempotency_id,correlation_id,operation_ordinal,command_id,command_expected_version,command_expected_current_path,command_new_vault_path,command_observed_fingerprint_algorithm,command_observed_fingerprint_digest,command_observed_at,result_version,audit_event_id) VALUES('relocate_evidence_reference','synthetic-v43-relocate','synthetic-v43-correlation',0,'synthetic-v43-evidence',1,'evidence/old.md','evidence/synthetic-v43.md','sha256','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',150,2,'synthetic-v43-audit');",
            )
            .unwrap_or_else(|error| {
                panic!("populated v43 evidence fixture must be recorded: {error}")
            });
        transaction
            .commit()
            .unwrap_or_else(|_| panic!("populated v43 fixture must commit"));
        connection
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn v43_to_v44_adds_the_pin_replay_authority_and_preserves_every_row_and_claim() {
        let mut connection = committed_v43_evidence_fixture();
        // Same procedure as the v43 test: the shipped Ledger has no in-place
        // upgrade path, so this is forward-runner evidence, run the way
        // SQLite documents schema changes over populated tables.
        connection
            .execute_batch("PRAGMA foreign_keys=OFF;")
            .unwrap();
        let transaction = connection.transaction().unwrap();
        apply_migration(&transaction, &MIGRATIONS[43], false)
            .unwrap_or_else(|error| panic!("v43 to v44 must succeed: {error:?}"));
        transaction.commit().unwrap();
        let dangling: i64 = connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(dangling, 0);
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();

        let count = |sql: &str| -> i64 { connection.query_row(sql, [], |row| row.get(0)).unwrap() };
        assert_eq!(count("SELECT count(*) FROM evidence_references WHERE id='synthetic-v43-evidence' AND fingerprint_algorithm='sha256' AND verification='verified'"), 1);
        assert_eq!(count("SELECT count(*) FROM evidence_relocation_command_results WHERE idempotency_id='synthetic-v43-relocate'"), 1);
        assert_eq!(count("SELECT count(*) FROM ledger_idempotency_claims WHERE idempotency_id='synthetic-v43-relocate' AND namespace='evidence'"), 1);
        assert_eq!(count("SELECT count(*) FROM sqlite_schema WHERE type='table' AND name='evidence_fingerprint_pin_command_results'"), 1);
        assert_eq!(count("SELECT count(*) FROM sqlite_schema WHERE type='trigger' AND name='ledger_idempotency_claim_evidence_fingerprint_pin' AND tbl_name='evidence_fingerprint_pin_command_results'"), 1);
        assert_eq!(
            count("SELECT count(*) FROM sqlite_schema WHERE name LIKE '%_v44'"),
            0
        );

        // The new table claims its idempotency key through the trigger...
        connection
            .execute(
                "INSERT INTO evidence_fingerprint_pin_command_results(operation,idempotency_id,correlation_id,operation_ordinal,command_id,command_expected_version,command_expected_current_path,command_observed_fingerprint_algorithm,command_observed_fingerprint_digest,command_observed_at,result_version,audit_event_id) VALUES('pin_evidence_fingerprint','synthetic-v44-pin','synthetic-v44-correlation',0,'synthetic-v43-evidence',2,'evidence/synthetic-v43.md','sha256',?1,160,3,'synthetic-v43-audit')",
                ["b".repeat(64)],
            )
            .expect("a pin command result must be admitted");
        assert_eq!(count("SELECT count(*) FROM ledger_idempotency_claims WHERE idempotency_id='synthetic-v44-pin' AND namespace='evidence' AND operation='pin_evidence_fingerprint'"), 1);
        // ...and a key another Evidence command already claimed is refused.
        assert!(
            connection
                .execute(
                    "INSERT INTO evidence_fingerprint_pin_command_results(operation,idempotency_id,correlation_id,operation_ordinal,command_id,command_expected_version,command_expected_current_path,command_observed_fingerprint_algorithm,command_observed_fingerprint_digest,command_observed_at,result_version,audit_event_id) VALUES('pin_evidence_fingerprint','synthetic-v43-relocate','synthetic-v44-correlation-2',1,'synthetic-v43-evidence',2,'evidence/synthetic-v43.md','sha256',?1,160,3,'synthetic-v43-audit')",
                    ["b".repeat(64)],
                )
                .is_err(),
            "an idempotency key claimed by relocation must not be reusable for a pin"
        );
    }

    /// A committed v45 Ledger holding one Open Risk and one Open Issue, each
    /// with an outstanding (unconsumed) H2a preview, plus the audit rows a
    /// later rejection would bind to. The rows are what v46's triggers judge.
    fn committed_v45_rejection_fixture() -> Connection {
        let mut connection = committed_v1_fixture();
        let transaction = connection
            .transaction()
            .unwrap_or_else(|_| panic!("synthetic v45 transaction"));
        for migration in &MIGRATIONS[1..45] {
            apply_migration(&transaction, migration, false)
                .unwrap_or_else(|_| panic!("v45 fixture migration must succeed"));
        }
        transaction
            .execute(
                "UPDATE ledger_metadata SET schema_version=45 WHERE singleton=1",
                [],
            )
            .unwrap_or_else(|_| panic!("v45 fixture metadata must be recorded"));
        transaction
            .execute_batch("PRAGMA user_version=45;")
            .unwrap_or_else(|_| panic!("v45 fixture identity must be recorded"));
        transaction
            .execute_batch(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES('synthetic-v45-risk','risk',1,'internal',100,100),('synthetic-v45-issue','issue',1,'internal',100,100);
                 INSERT INTO risks(id,title,details,state,response,owner_id,rationale,residual_exposure,next_review_at,in_exception_queue) VALUES('synthetic-v45-risk','Synthetic v45 risk','Synthetic v45 risk details','open',NULL,NULL,NULL,NULL,NULL,0);
                 INSERT INTO issues(id,source_risk_id,recurrence_of_id,title,details,state,resolution_type,resolution_rationale) VALUES('synthetic-v45-issue',NULL,NULL,'Synthetic v45 issue','Synthetic v45 issue details','open',NULL,NULL);
                 INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at,consumed_at) VALUES('synthetic-v45-risk-prepared',1,'record_risk_occurrence','synthetic-v45-risk-digest','internal','allowed','not_cancellable_after_submit','head_of_products',NULL,NULL,300100,100,NULL),('synthetic-v45-issue-prepared',1,'resolve_issue','synthetic-v45-issue-digest','internal','allowed','not_cancellable_after_submit','head_of_products',NULL,NULL,300100,100,NULL);
                 INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES('synthetic-v46-risk-reject-audit',180,'head_of_products','work_management','risk.prepared_rejected','risk','synthetic-v45-risk','synthetic-v46-risk-reject-correlation','allowed','rejected','not_attempted','none'),('synthetic-v46-risk-wrong-audit',190,'head_of_products','work_management','risk.prepared_rejected','risk','synthetic-v45-risk','synthetic-v46-risk-wrong-correlation','allowed','rejected','not_attempted','none'),('synthetic-v46-issue-reject-audit',190,'head_of_products','work_management','issue.prepared_rejected','issue','synthetic-v45-issue','synthetic-v46-issue-reject-correlation','allowed','rejected','not_attempted','none');",
            )
            .unwrap_or_else(|error| panic!("populated v45 rejection fixture must be recorded: {error}"));
        transaction
            .commit()
            .unwrap_or_else(|_| panic!("populated v45 fixture must commit"));
        connection
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn v45_to_v46_adds_independent_rejection_streams_that_admit_only_consuming_rejections() {
        let mut connection = committed_v45_rejection_fixture();
        connection
            .execute_batch("PRAGMA foreign_keys=OFF;")
            .unwrap();
        let transaction = connection.transaction().unwrap();
        apply_migration(&transaction, &MIGRATIONS[45], false)
            .unwrap_or_else(|error| panic!("v45 to v46 must succeed: {error:?}"));
        transaction.commit().unwrap();
        let dangling: i64 = connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(dangling, 0);
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();

        let count = |sql: &str| -> i64 { connection.query_row(sql, [], |row| row.get(0)).unwrap() };
        // No staging table: nothing was recreated.
        assert_eq!(
            count("SELECT count(*) FROM sqlite_schema WHERE type='table' AND name LIKE '%\\_v46' ESCAPE '\\'"),
            0
        );
        for (kind, name) in [
            ("table", "risk_reject_prepared_command_results"),
            ("table", "issue_reject_prepared_command_results"),
            (
                "trigger",
                "risk_reject_prepared_command_results_contiguous_insert",
            ),
            (
                "trigger",
                "risk_reject_prepared_command_results_ordinal_immutable",
            ),
            ("trigger", "risk_reject_prepared_binding_insert"),
            ("trigger", "risk_reject_prepared_binding_update"),
            ("trigger", "ledger_idempotency_claim_risk_reject_prepared"),
            (
                "trigger",
                "issue_reject_prepared_command_results_contiguous_insert",
            ),
            (
                "trigger",
                "issue_reject_prepared_command_results_ordinal_immutable",
            ),
            ("trigger", "issue_reject_prepared_binding_insert"),
            ("trigger", "issue_reject_prepared_binding_update"),
            ("trigger", "ledger_idempotency_claim_issue_reject_prepared"),
            ("index", "idx_risk_reject_prepared_ordinal"),
            ("index", "idx_risk_reject_prepared_correlation"),
            ("index", "idx_issue_reject_prepared_ordinal"),
            ("index", "idx_issue_reject_prepared_correlation"),
        ] {
            assert_eq!(
                count(&format!(
                    "SELECT count(*) FROM sqlite_schema WHERE type='{kind}' AND name='{name}'"
                )),
                1,
                "{name}"
            );
        }

        // A rejection whose prepared intent is still unconsumed is refused...
        assert!(
            connection
                .execute(
                    "INSERT INTO risk_reject_prepared_command_results(operation,idempotency_id,correlation_id,operation_ordinal,prepared_intent_id,actor,rejected_at,audit_event_id) VALUES('reject_prepared','synthetic-v46-risk-reject','synthetic-v46-risk-reject-correlation',0,'synthetic-v45-risk-prepared','head_of_products',180,'synthetic-v46-risk-reject-audit')",
                    [],
                )
                .is_err(),
            "a Risk rejection whose prepared intent is still unconsumed must be refused"
        );
        // ...and one that consumed it at the same instant is admitted.
        connection
            .execute(
                "UPDATE prepared_intents SET consumed_at=180 WHERE id='synthetic-v45-risk-prepared'",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO risk_reject_prepared_command_results(operation,idempotency_id,correlation_id,operation_ordinal,prepared_intent_id,actor,rejected_at,audit_event_id) VALUES('reject_prepared','synthetic-v46-risk-reject','synthetic-v46-risk-reject-correlation',0,'synthetic-v45-risk-prepared','head_of_products',180,'synthetic-v46-risk-reject-audit')",
                [],
            )
            .expect("a consuming Risk rejection must be admitted");
        // It claims its idempotency key in its own namespace...
        assert_eq!(
            count("SELECT count(*) FROM ledger_idempotency_claims WHERE idempotency_id='synthetic-v46-risk-reject' AND namespace='risk' AND operation='reject_prepared'"),
            1
        );
        // ...and can be neither re-pointed nor renumbered.
        assert!(connection.execute("UPDATE risk_reject_prepared_command_results SET rejected_at=181 WHERE idempotency_id='synthetic-v46-risk-reject'", []).is_err());
        assert!(connection.execute("UPDATE risk_reject_prepared_command_results SET operation_ordinal=7 WHERE idempotency_id='synthetic-v46-risk-reject'", []).is_err());

        // A Risk rejection cannot consume an Issue intent, even a consumed one.
        connection
            .execute(
                "UPDATE prepared_intents SET consumed_at=190 WHERE id='synthetic-v45-issue-prepared'",
                [],
            )
            .unwrap();
        assert!(
            connection
                .execute(
                    "INSERT INTO risk_reject_prepared_command_results(operation,idempotency_id,correlation_id,operation_ordinal,prepared_intent_id,actor,rejected_at,audit_event_id) VALUES('reject_prepared','synthetic-v46-risk-wrong','synthetic-v46-risk-wrong-correlation',1,'synthetic-v45-issue-prepared','head_of_products',190,'synthetic-v46-risk-wrong-audit')",
                    [],
                )
                .is_err(),
            "a Risk rejection must not consume an Issue prepared intent"
        );

        // The Issue stream is its own: it starts at zero regardless of the
        // Risk stream, and refuses a gap.
        assert!(
            connection
                .execute(
                    "INSERT INTO issue_reject_prepared_command_results(operation,idempotency_id,correlation_id,operation_ordinal,prepared_intent_id,actor,rejected_at,audit_event_id) VALUES('reject_prepared','synthetic-v46-issue-reject','synthetic-v46-issue-reject-correlation',1,'synthetic-v45-issue-prepared','head_of_products',190,'synthetic-v46-issue-reject-audit')",
                    [],
                )
                .is_err(),
            "the Issue rejection stream must be contiguous from zero"
        );
        connection
            .execute(
                "INSERT INTO issue_reject_prepared_command_results(operation,idempotency_id,correlation_id,operation_ordinal,prepared_intent_id,actor,rejected_at,audit_event_id) VALUES('reject_prepared','synthetic-v46-issue-reject','synthetic-v46-issue-reject-correlation',0,'synthetic-v45-issue-prepared','head_of_products',190,'synthetic-v46-issue-reject-audit')",
                [],
            )
            .expect("a consuming Issue rejection must be admitted at ordinal zero");
        assert_eq!(
            count("SELECT count(*) FROM ledger_idempotency_claims WHERE idempotency_id='synthetic-v46-issue-reject' AND namespace='issue' AND operation='reject_prepared'"),
            1
        );
        // The metadata row now names v46 with the recorded checksum.
        assert_eq!(
            count("SELECT count(*) FROM schema_migrations WHERE version=46 AND migration_key='0046_risk_issue_prepared_rejection'"),
            1
        );
    }

    /// A committed v46 Ledger holding one Open Risk: the row a v47 response
    /// update and reservation would bind to.
    fn committed_v46_risk_fixture() -> Connection {
        let mut connection = committed_v1_fixture();
        let transaction = connection
            .transaction()
            .unwrap_or_else(|_| panic!("synthetic v46 transaction"));
        for migration in &MIGRATIONS[1..46] {
            apply_migration(&transaction, migration, false)
                .unwrap_or_else(|_| panic!("v46 fixture migration must succeed"));
        }
        transaction
            .execute(
                "UPDATE ledger_metadata SET schema_version=46 WHERE singleton=1",
                [],
            )
            .unwrap_or_else(|_| panic!("v46 fixture metadata must be recorded"));
        transaction
            .execute_batch("PRAGMA user_version=46;")
            .unwrap_or_else(|_| panic!("v46 fixture identity must be recorded"));
        transaction
            .execute_batch(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES('synthetic-v46-risk','risk',1,'internal',100,100);
                 INSERT INTO risks(id,title,details,state,response,owner_id,rationale,residual_exposure,next_review_at,in_exception_queue) VALUES('synthetic-v46-risk','Synthetic v46 risk','Synthetic v46 risk details','open',NULL,NULL,NULL,NULL,NULL,1);
                 INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES('synthetic-v47-response-audit',200,'head_of_products','work_management','risk.response_updated','risk','synthetic-v46-risk','synthetic-v47-response-correlation','allowed','not_required','succeeded','complete');",
            )
            .unwrap_or_else(|error| panic!("populated v46 risk fixture must be recorded: {error}"));
        transaction
            .commit()
            .unwrap_or_else(|_| panic!("populated v46 fixture must commit"));
        connection
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn v46_to_v47_adds_reservations_and_the_risk_response_replay_authority() {
        let mut connection = committed_v46_risk_fixture();
        connection
            .execute_batch("PRAGMA foreign_keys=OFF;")
            .unwrap();
        let transaction = connection.transaction().unwrap();
        apply_migration(&transaction, &MIGRATIONS[46], false)
            .unwrap_or_else(|error| panic!("v46 to v47 must succeed: {error:?}"));
        transaction.commit().unwrap();
        let dangling: i64 = connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(dangling, 0);
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();

        let count = |sql: &str| -> i64 { connection.query_row(sql, [], |row| row.get(0)).unwrap() };
        // No staging table: nothing was recreated; the Risk is as it was.
        assert_eq!(
            count("SELECT count(*) FROM sqlite_schema WHERE type='table' AND name LIKE '%\\_v47' ESCAPE '\\'"),
            0
        );
        assert_eq!(count("SELECT count(*) FROM risks WHERE id='synthetic-v46-risk' AND state='open' AND response IS NULL"), 1);
        for (kind, name) in [
            ("table", "record_id_reservations"),
            ("trigger", "record_id_reservations_immutable"),
            ("trigger", "record_id_reservations_no_delete"),
            ("table", "risk_response_replay_operations"),
            ("table", "risk_response_command_updates"),
            ("table", "risk_response_replay_audits"),
            ("trigger", "ledger_idempotency_claim_risk_response"),
            (
                "trigger",
                "risk_response_replay_operations_contiguous_insert",
            ),
            (
                "trigger",
                "risk_response_replay_operations_ordinal_immutable",
            ),
            ("index", "idx_risk_response_replay_ordinal"),
            ("index", "idx_risk_response_replay_correlation"),
            ("index", "idx_risk_response_replay_risk"),
        ] {
            assert_eq!(
                count(&format!(
                    "SELECT count(*) FROM sqlite_schema WHERE type='{kind}' AND name='{name}'"
                )),
                1,
                "{name}"
            );
        }

        // A reservation is written once and then neither changed nor removed.
        connection
            .execute(
                "INSERT INTO record_id_reservations(idempotency_id,namespace,operation,entity_kind,generated_id,reserved_at) VALUES('synthetic-sheet','risk','create_risk','risk','risk-reserved',300)",
                [],
            )
            .expect("a reservation must be admitted");
        assert!(connection.execute("UPDATE record_id_reservations SET generated_id='risk-other' WHERE idempotency_id='synthetic-sheet'", []).is_err());
        assert!(connection
            .execute(
                "DELETE FROM record_id_reservations WHERE idempotency_id='synthetic-sheet'",
                []
            )
            .is_err());
        assert!(
            connection.execute("INSERT INTO record_id_reservations(idempotency_id,namespace,operation,entity_kind,generated_id,reserved_at) VALUES('synthetic-sheet-2','risk','create_risk','risk','risk-reserved',300)", []).is_err(),
            "one generated id serves one reservation"
        );
        // Reserving claims nothing: the command's own replay row does that.
        assert_eq!(count("SELECT count(*) FROM ledger_idempotency_claims WHERE idempotency_id='synthetic-sheet'"), 0);

        // An Accept result without its attributes is refused by the table...
        assert!(
            connection.execute(
                "INSERT INTO risk_response_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_reference,result_classification,result_response,result_owner_id,result_rationale,result_residual_exposure,result_next_review_at,result_in_exception_queue,result_version) VALUES('synthetic-v47-response','update_risk_response','synthetic-v47-response-correlation',0,'synthetic-v46-risk','internal','accept',NULL,NULL,NULL,NULL,0,2)",
                [],
            ).is_err()
        );
        // ...a Mitigate result at ordinal zero is admitted and claims its key...
        connection
            .execute(
                "INSERT INTO risk_response_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_reference,result_classification,result_response,result_owner_id,result_rationale,result_residual_exposure,result_next_review_at,result_in_exception_queue,result_version) VALUES('synthetic-v47-response','update_risk_response','synthetic-v47-response-correlation',0,'synthetic-v46-risk','internal','mitigate',NULL,NULL,NULL,NULL,1,2)",
                [],
            )
            .expect("a response result must be admitted at ordinal zero");
        assert_eq!(count("SELECT count(*) FROM ledger_idempotency_claims WHERE idempotency_id='synthetic-v47-response' AND namespace='risk' AND operation='update_risk_response'"), 1);
        connection
            .execute(
                "INSERT INTO risk_response_replay_audits(idempotency_id,audit_event_id,correlation_id) VALUES('synthetic-v47-response','synthetic-v47-response-audit','synthetic-v47-response-correlation')",
                [],
            )
            .expect("the audit binding must be admitted");
        // ...and can be neither renumbered nor followed by a gap.
        assert!(connection.execute("UPDATE risk_response_replay_operations SET operation_ordinal=5 WHERE idempotency_id='synthetic-v47-response'", []).is_err());
        assert!(
            connection.execute(
                "INSERT INTO risk_response_replay_operations(idempotency_id,operation,correlation_id,operation_ordinal,result_reference,result_classification,result_response,result_owner_id,result_rationale,result_residual_exposure,result_next_review_at,result_in_exception_queue,result_version) VALUES('synthetic-v47-response-2','update_risk_response','synthetic-v47-response-correlation-2',2,'synthetic-v46-risk','internal','avoid',NULL,NULL,NULL,NULL,1,3)",
                [],
            ).is_err(),
            "the response stream must be contiguous from zero"
        );
        // The metadata row now names v47 with the recorded checksum.
        assert_eq!(
            count("SELECT count(*) FROM schema_migrations WHERE version=47 AND migration_key='0047_record_entry_foundation'"),
            1
        );
    }

    fn committed_v44_h2a_fixture() -> Connection {
        let mut connection = committed_v1_fixture();
        let transaction = connection
            .transaction()
            .unwrap_or_else(|_| panic!("synthetic v44 transaction"));
        for migration in &MIGRATIONS[1..44] {
            apply_migration(&transaction, migration, false)
                .unwrap_or_else(|_| panic!("v44 fixture migration must succeed"));
        }
        transaction
            .execute(
                "UPDATE ledger_metadata SET schema_version=44 WHERE singleton=1",
                [],
            )
            .unwrap_or_else(|_| panic!("v44 fixture metadata must be recorded"));
        transaction
            .execute_batch("PRAGMA user_version=44;")
            .unwrap_or_else(|_| panic!("v44 fixture identity must be recorded"));
        // One Evidence reference at version 3 whose verification was recorded
        // at version 1 (path 'evidence/before.md'), then relocated; one
        // support witness + Complete-Action Prepared Intent whose snapshot
        // row predates the evidence_version column.
        transaction
            .execute_batch(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES('synthetic-v44-evidence','evidence_reference',3,'internal',100,170);
                 INSERT INTO evidence_references(id,role,verification,integrity_digest,last_verified_at,vault_relative_path,fingerprint_algorithm,fingerprint_digest,provenance_kind,provenance_reference) VALUES('synthetic-v44-evidence',NULL,'verified','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',150,'evidence/after.md','sha256','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','synthetic_fixture','migration-test');
                 INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES('synthetic-v44-audit',150,'head_of_products','work_management','evidence.verification_updated','evidence_reference','synthetic-v44-evidence','synthetic-v44-correlation','allowed','not_required','succeeded','complete');
                 INSERT INTO evidence_verification_command_results(operation,idempotency_id,correlation_id,operation_ordinal,command_id,command_expected_version,command_verification,command_last_verified_at,command_integrity_digest,result_version,audit_event_id) VALUES('update_evidence_verification','synthetic-v44-verify','synthetic-v44-correlation',0,'synthetic-v44-evidence',1,'verified',150,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',2,'synthetic-v44-audit');
                 INSERT INTO support_witnesses(id,requirement,disposition,classification) VALUES('synthetic-v44-support','evidence_required','evidence_satisfied','internal');
                 INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at,consumed_at) VALUES('synthetic-v44-prepared',1,'complete_action','synthetic-v44-digest','internal','allowed','not_cancellable_after_submit','head_of_products','synthetic-v44-support',NULL,400,160,NULL);
                 INSERT INTO action_h2a_support_evidence_snapshots(prepared_intent_id,ordinal,evidence_id,classification,role,verification,last_verified_at,integrity_digest) VALUES('synthetic-v44-prepared',0,'synthetic-v44-evidence','internal','action_completion','verified',150,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');",
            )
            .unwrap_or_else(|error| panic!("populated v44 H2a fixture must be recorded: {error}"));
        transaction
            .commit()
            .unwrap_or_else(|_| panic!("populated v44 fixture must commit"));
        connection
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn v44_to_v45_binds_evidence_versions_and_paths_and_admits_only_consuming_rejections() {
        let mut connection = committed_v44_h2a_fixture();
        connection
            .execute_batch("PRAGMA foreign_keys=OFF;")
            .unwrap();
        let transaction = connection.transaction().unwrap();
        apply_migration(&transaction, &MIGRATIONS[44], false)
            .unwrap_or_else(|error| panic!("v44 to v45 must succeed: {error:?}"));
        transaction.commit().unwrap();
        let dangling: i64 = connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(dangling, 0);
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();

        let count = |sql: &str| -> i64 { connection.query_row(sql, [], |row| row.get(0)).unwrap() };
        // No `_v45` staging table survives (the v45 contiguity triggers are
        // named `*_contiguous_v45` on purpose, so this looks at tables only).
        assert_eq!(
            count("SELECT count(*) FROM sqlite_schema WHERE type='table' AND name LIKE '%\\_v45' ESCAPE '\\'"),
            0
        );
        // The verification row now carries the path it was recorded
        // against, backfilled from the live path (exact only for a bootstrap).
        assert_eq!(count("SELECT count(*) FROM evidence_verification_command_results WHERE idempotency_id='synthetic-v44-verify' AND command_vault_path='evidence/after.md' AND command_expected_version=1 AND result_version=2"), 1);
        assert_eq!(count("SELECT count(*) FROM ledger_idempotency_claims WHERE idempotency_id='synthetic-v44-verify' AND namespace='evidence'"), 1);
        assert_eq!(count("SELECT count(*) FROM sqlite_schema WHERE type='trigger' AND name='ledger_idempotency_claim_evidence_verification' AND tbl_name='evidence_verification_command_results'"), 1);
        assert!(connection.execute("UPDATE evidence_verification_command_results SET command_vault_path='' WHERE idempotency_id='synthetic-v44-verify'", []).is_err());
        // The snapshot row binds the registry version it was backfilled from.
        assert_eq!(count("SELECT count(*) FROM action_h2a_support_evidence_snapshots WHERE prepared_intent_id='synthetic-v44-prepared' AND evidence_id='synthetic-v44-evidence' AND evidence_version=3 AND verification='verified'"), 1);
        assert!(connection.execute("UPDATE action_h2a_support_evidence_snapshots SET evidence_version=0 WHERE prepared_intent_id='synthetic-v44-prepared'", []).is_err());
        for name in [
            "idx_action_h2a_support_evidence_snapshot_evidence",
            "idx_decision_h2a_support_evidence_snapshot_evidence",
            "idx_issue_h2a_support_evidence_snapshot_evidence",
        ] {
            assert_eq!(
                count(&format!(
                    "SELECT count(*) FROM sqlite_schema WHERE type='index' AND name='{name}'"
                )),
                1,
                "{name}"
            );
        }
        for stale in [
            "action_replay_operations_contiguous_v36",
            "decision_replay_operations_contiguous_global_v27",
        ] {
            assert_eq!(
                count(&format!(
                    "SELECT count(*) FROM sqlite_schema WHERE type='trigger' AND name='{stale}'"
                )),
                0,
                "{stale}"
            );
        }
        for fresh in [
            "action_replay_operations_contiguous_v45",
            "action_reject_prepared_command_results_contiguous_v45",
            "decision_replay_operations_contiguous_v45",
            "decision_reject_prepared_command_results_contiguous_v45",
        ] {
            assert_eq!(
                count(&format!(
                    "SELECT count(*) FROM sqlite_schema WHERE type='trigger' AND name='{fresh}'"
                )),
                1,
                "{fresh}"
            );
        }
        // A rejection row must consume its prepared intent at the same instant...
        assert!(
            connection
                .execute(
                    "INSERT INTO action_reject_prepared_command_results(operation,idempotency_id,correlation_id,operation_ordinal,prepared_intent_id,actor,rejected_at,audit_event_id) VALUES('reject_prepared','synthetic-v45-reject','synthetic-v44-correlation',0,'synthetic-v44-prepared','head_of_products',180,'synthetic-v44-audit')",
                    [],
                )
                .is_err(),
            "a rejection whose prepared intent is still unconsumed must be refused"
        );
        connection
            .execute(
                "UPDATE prepared_intents SET consumed_at=180 WHERE id='synthetic-v44-prepared'",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO action_reject_prepared_command_results(operation,idempotency_id,correlation_id,operation_ordinal,prepared_intent_id,actor,rejected_at,audit_event_id) VALUES('reject_prepared','synthetic-v45-reject','synthetic-v44-correlation',0,'synthetic-v44-prepared','head_of_products',180,'synthetic-v44-audit')",
                [],
            )
            .expect("a consuming rejection must be admitted");
        // ...claims its idempotency key...
        assert_eq!(count("SELECT count(*) FROM ledger_idempotency_claims WHERE idempotency_id='synthetic-v45-reject' AND namespace='action' AND operation='reject_prepared'"), 1);
        // ...and can neither be re-pointed nor renumbered.
        assert!(connection.execute("UPDATE action_reject_prepared_command_results SET rejected_at=181 WHERE idempotency_id='synthetic-v45-reject'", []).is_err());
        assert!(connection.execute("UPDATE action_reject_prepared_command_results SET operation_ordinal=7 WHERE idempotency_id='synthetic-v45-reject'", []).is_err());
        // A Decision rejection cannot consume an Action intent.
        assert!(
            connection
                .execute(
                    "INSERT INTO decision_reject_prepared_command_results(operation,idempotency_id,correlation_id,operation_ordinal,prepared_intent_id,actor,rejected_at,audit_event_id) VALUES('reject_prepared','synthetic-v45-reject-2','synthetic-v44-correlation',0,'synthetic-v44-prepared','head_of_products',180,'synthetic-v44-audit')",
                    [],
                )
                .is_err()
        );
    }

    fn committed_v42_evidence_fixture() -> Connection {
        let mut connection = committed_v1_fixture();
        let transaction = connection
            .transaction()
            .unwrap_or_else(|_| panic!("synthetic v42 transaction"));
        for migration in &MIGRATIONS[1..42] {
            apply_migration(&transaction, migration, false)
                .unwrap_or_else(|_| panic!("v42 fixture migration must succeed"));
        }
        transaction
            .execute(
                "UPDATE ledger_metadata SET schema_version=42 WHERE singleton=1",
                [],
            )
            .unwrap_or_else(|_| panic!("v42 fixture metadata must be recorded"));
        transaction
            .execute_batch("PRAGMA user_version=42;")
            .unwrap_or_else(|_| panic!("v42 fixture identity must be recorded"));
        transaction
            .execute_batch(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES('synthetic-v42-evidence','evidence_reference',2,'internal',100,150);
                 INSERT INTO evidence_references(id,role,verification,integrity_digest,last_verified_at,vault_relative_path,fingerprint_algorithm,fingerprint_digest,provenance_kind,provenance_reference) VALUES('synthetic-v42-evidence','action_completion','verified','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',150,'evidence/synthetic-v42.md',NULL,NULL,'synthetic_fixture','migration-test');
                 INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES('synthetic-v42-audit',100,'head_of_products','work_management','evidence_reference.created','evidence_reference','synthetic-v42-evidence','synthetic-v42-correlation','allowed','not_required','succeeded','complete');
                 INSERT INTO evidence_reference_command_results(operation,idempotency_id,correlation_id,operation_ordinal,command_id,command_vault_relative_path,command_fingerprint_algorithm,command_fingerprint_digest,command_verification,command_last_verified_at,command_integrity_digest,command_classification,command_provenance_kind,command_provenance_reference,result_classification,result_created_at,audit_event_id) VALUES('create_evidence_reference','synthetic-v42-create','synthetic-v42-correlation',0,'synthetic-v42-evidence','evidence/synthetic-v42.md',NULL,NULL,'unverified',NULL,NULL,'internal','synthetic_fixture','migration-test','internal',100,'synthetic-v42-audit');
                 INSERT INTO evidence_verification_command_results(operation,idempotency_id,correlation_id,operation_ordinal,command_id,command_expected_version,command_verification,command_last_verified_at,command_integrity_digest,result_version,audit_event_id) VALUES('update_evidence_verification','synthetic-v42-verify','synthetic-v42-correlation-verify',0,'synthetic-v42-evidence',1,'verified',150,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',2,'synthetic-v42-audit');
                 INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,support_id,confirmation_challenge,expires_at,created_at,consumed_at) VALUES('synthetic-v42-prepared',1,'complete_action','synthetic-v42-digest','internal','allowed','not_cancellable_after_submit','head_of_products',NULL,NULL,200,100,NULL),('synthetic-v42-prepared-2',1,'complete_action','synthetic-v42-digest-2','internal','allowed','not_cancellable_after_submit','head_of_products',NULL,NULL,200,100,NULL),('synthetic-v42-prepared-3',1,'complete_action','synthetic-v42-digest-3','internal','allowed','not_cancellable_after_submit','head_of_products',NULL,NULL,200,100,NULL);
                 INSERT INTO action_h2a_support_evidence_snapshots(prepared_intent_id,ordinal,evidence_id,classification,role,verification,last_verified_at,integrity_digest) VALUES('synthetic-v42-prepared',0,'synthetic-v42-evidence','internal','action_completion','verified',150,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');
                 INSERT INTO issue_h2a_support_evidence_snapshots(prepared_intent_id,ordinal,evidence_id,classification,role,verification,last_verified_at,integrity_digest) VALUES('synthetic-v42-prepared',0,'synthetic-v42-evidence','internal','resolution','degraded_last_verified',150,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');
                 INSERT INTO decision_h2a_support_evidence_snapshots(prepared_intent_id,ordinal,evidence_id,classification,role,verification,last_verified_at,integrity_digest) VALUES('synthetic-v42-prepared',0,'synthetic-v42-evidence','internal','decision_resolution','unverified',NULL,NULL);",
            )
            .unwrap_or_else(|error| {
                panic!("populated v42 evidence fixture must be recorded: {error}")
            });
        transaction
            .commit()
            .unwrap_or_else(|_| panic!("populated v42 fixture must commit"));
        connection
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn v42_to_v43_preserves_every_verification_row_and_admits_observed_unpinned_only_with_its_parts(
    ) {
        let mut connection = committed_v42_evidence_fixture();
        // The shipped Ledger has no in-place upgrade path: `open` refuses any
        // other schema version, and migrations run only at bootstrap on empty
        // tables. This fixture runs the SQL against populated tables anyway,
        // because that is what the recreate must preserve, and it does so the
        // way SQLite documents table recreation: foreign keys off around the
        // transaction (a referenced parent cannot be dropped with them on,
        // deferred or not), then `foreign_key_check` to prove nothing dangles.
        // Any future upgrade runner must do the same.
        connection
            .execute_batch("PRAGMA foreign_keys=OFF;")
            .unwrap();
        let transaction = connection.transaction().unwrap();
        apply_migration(&transaction, &MIGRATIONS[42], false)
            .unwrap_or_else(|error| panic!("v42 to v43 must succeed: {error:?}"));
        transaction.commit().unwrap();
        let dangling: i64 = connection
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            dangling, 0,
            "the recreated tables must satisfy every child reference"
        );
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();

        let count = |sql: &str| -> i64 { connection.query_row(sql, [], |row| row.get(0)).unwrap() };
        assert_eq!(count("SELECT count(*) FROM evidence_references WHERE id='synthetic-v42-evidence' AND verification='verified' AND last_verified_at=150 AND role='action_completion' AND vault_relative_path='evidence/synthetic-v42.md'"), 1);
        assert_eq!(count("SELECT count(*) FROM evidence_reference_command_results WHERE idempotency_id='synthetic-v42-create' AND command_verification='unverified' AND audit_event_id='synthetic-v42-audit'"), 1);
        assert_eq!(count("SELECT count(*) FROM evidence_verification_command_results WHERE idempotency_id='synthetic-v42-verify' AND command_verification='verified' AND result_version=2"), 1);
        assert_eq!(count("SELECT count(*) FROM action_h2a_support_evidence_snapshots WHERE prepared_intent_id='synthetic-v42-prepared' AND verification='verified'"), 1);
        assert_eq!(count("SELECT count(*) FROM issue_h2a_support_evidence_snapshots WHERE prepared_intent_id='synthetic-v42-prepared' AND verification='degraded_last_verified'"), 1);
        assert_eq!(count("SELECT count(*) FROM decision_h2a_support_evidence_snapshots WHERE prepared_intent_id='synthetic-v42-prepared' AND verification='unverified'"), 1);
        // Claims the triggers recorded before the migration survive it, and
        // the triggers and indexes are attached to the recreated tables.
        assert_eq!(count("SELECT count(*) FROM ledger_idempotency_claims WHERE idempotency_id IN ('synthetic-v42-create','synthetic-v42-verify') AND namespace='evidence'"), 2);
        assert_eq!(count("SELECT count(*) FROM sqlite_schema WHERE type='trigger' AND tbl_name IN ('evidence_reference_command_results','evidence_verification_command_results') AND name IN ('ledger_idempotency_claim_evidence_reference','ledger_idempotency_claim_evidence_verification')"), 2);
        assert_eq!(count("SELECT count(*) FROM sqlite_schema WHERE type='index' AND name IN ('idx_action_h2a_support_evidence_snapshot_evidence','idx_issue_h2a_support_evidence_snapshot_evidence','idx_decision_h2a_support_evidence_snapshot_evidence')"), 3);
        assert_eq!(
            count("SELECT count(*) FROM sqlite_schema WHERE type='table' AND name LIKE '%_v43'"),
            0
        );

        // The fifth value is admitted with its digest and time...
        connection
            .execute(
                "UPDATE evidence_references SET verification='observed_unpinned',integrity_digest=?1,last_verified_at=160 WHERE id='synthetic-v42-evidence'",
                ["b".repeat(64)],
            )
            .expect("observed_unpinned with a digest and time must be admitted");
        connection
            .execute(
                "INSERT INTO issue_h2a_support_evidence_snapshots(prepared_intent_id,ordinal,evidence_id,classification,role,verification,last_verified_at,integrity_digest) VALUES('synthetic-v42-prepared-2',0,'synthetic-v42-evidence','internal','resolution','observed_unpinned',160,?1)",
                ["b".repeat(64)],
            )
            .expect("an observed_unpinned snapshot with its parts must be admitted");
        // ...and refused without them, exactly as verified and degraded are.
        assert!(
            connection
                .execute(
                    "INSERT INTO issue_h2a_support_evidence_snapshots(prepared_intent_id,ordinal,evidence_id,classification,role,verification,last_verified_at,integrity_digest) VALUES('synthetic-v42-prepared-3',0,'synthetic-v42-evidence','internal','resolution','observed_unpinned',NULL,NULL)",
                    [],
                )
                .is_err(),
            "observed_unpinned without its digest and time must be refused"
        );
        assert!(
            connection
                .execute(
                    "INSERT INTO evidence_verification_command_results(operation,idempotency_id,correlation_id,operation_ordinal,command_id,command_expected_version,command_verification,command_last_verified_at,command_integrity_digest,result_version,audit_event_id) VALUES('update_evidence_verification','synthetic-v42-verify-2','synthetic-v42-correlation-verify-2',1,'synthetic-v42-evidence',2,'observed_unpinned',NULL,NULL,3,'synthetic-v42-audit')",
                    [],
                )
                .is_err(),
            "an observed_unpinned command result without its parts must be refused"
        );
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn v1_to_current_test_migration_succeeds_and_preserves_synthetic_sentinel() {
        let mut connection = committed_v1_fixture();
        let transaction = connection
            .transaction()
            .unwrap_or_else(|_| panic!("synthetic v2 transaction"));
        migrate_v1_to_current_for_test(&transaction, false)
            .unwrap_or_else(|_| panic!("synthetic v1 to current migration must succeed"));
        transaction
            .commit()
            .unwrap_or_else(|_| panic!("synthetic v2 transaction must commit"));

        let expected = Connection::open_in_memory().unwrap();
        expected.execute_batch(V1_SCHEMA_SQL).unwrap();
        expected.execute_batch(V2_ACTION_SQL).unwrap();
        expected.execute_batch(V3_DECISION_H1_SQL).unwrap();
        expected.execute_batch(V4_DECISION_H2A_RESOLVE_SQL).unwrap();
        expected
            .execute_batch(V5_DECISION_REPLAY_GLOBAL_ORDINAL_SQL)
            .unwrap();
        expected
            .execute_batch(V6_DECISION_H2A_EVIDENCE_SNAPSHOT_SQL)
            .unwrap();
        expected.execute_batch(V7_RISK_H1_SQL).unwrap();
        expected
            .execute_batch(V8_DECISION_H2A_CORRELATION_ANCHOR_SQL)
            .unwrap();
        expected.execute_batch(V9_RISK_H2A_SQL).unwrap();
        expected.execute_batch(V10_ISSUE_H1_SQL).unwrap();
        expected
            .execute_batch(V11_RISK_H2A_TERMINAL_DENIAL_SQL)
            .unwrap();
        expected
            .execute_batch(V12_RISK_H2A_TERMINAL_DENIAL_AUDIT_BINDING_SQL)
            .unwrap();
        expected.execute_batch(V13_RISK_H2A_PREPARE_SQL).unwrap();
        expected.execute_batch(V14_RISK_H2A_EXECUTE_SQL).unwrap();
        expected.execute_batch(V15_ISSUE_H2A_PREPARE_SQL).unwrap();
        expected.execute_batch(V16_ISSUE_H2A_EXECUTE_SQL).unwrap();
        expected
            .execute_batch(V17_PORTFOLIO_KPI_OBSERVATION_INHERITED_CLASSIFICATION_SQL)
            .unwrap();
        expected
            .execute_batch(V18_EVIDENCE_REFERENCE_GENERALIZE_SQL)
            .unwrap();
        expected.execute_batch(V19_EVIDENCE_LINK_SQL).unwrap();
        expected
            .execute_batch(V20_EVIDENCE_VERIFICATION_UPDATE_SQL)
            .unwrap();
        expected
            .execute_batch(V21_PORTFOLIO_H2A_LOWER_CLASSIFICATION_SQL)
            .unwrap();
        expected
            .execute_batch(V22_PORTFOLIO_SIBLINGS_H2A_LOWER_CLASSIFICATION_SQL)
            .unwrap();
        expected
            .execute_batch(V23_ACTION_DECISION_RISK_ISSUE_H2A_LOWER_CLASSIFICATION_SQL)
            .unwrap();
        expected
            .execute_batch(V24_RISK_H2A_LOWER_CLASSIFICATION_PREPARE_SQL)
            .unwrap();
        expected
            .execute_batch(V25_RISK_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL)
            .unwrap();
        expected
            .execute_batch(V26_DECISION_H2A_LOWER_CLASSIFICATION_PREPARE_SQL)
            .unwrap();
        expected
            .execute_batch(V27_DECISION_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL)
            .unwrap();
        expected
            .execute_batch(V28_ISSUE_H2A_LOWER_CLASSIFICATION_PREPARE_SQL)
            .unwrap();
        expected
            .execute_batch(V29_ISSUE_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL)
            .unwrap();
        expected
            .execute_batch(V30_ACTION_H2A_LOWER_CLASSIFICATION_PREPARE_SQL)
            .unwrap();
        expected
            .execute_batch(V31_ACTION_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL)
            .unwrap();
        expected
            .execute_batch(V32_DELIVERY_H2A_LOWER_CLASSIFICATION_PREPARED_INTENT_KINDS_SQL)
            .unwrap();
        expected
            .execute_batch(V33_DELIVERY_H2A_LOWER_CLASSIFICATION_PREPARE_SQL)
            .unwrap();
        expected
            .execute_batch(V34_DELIVERY_H2A_LOWER_CLASSIFICATION_EXECUTE_SQL)
            .unwrap();
        expected
            .execute_batch(V35_DELIVERY_H2A_LOWER_CLASSIFICATION_EXECUTE_RESULT_SNAPSHOT_SQL)
            .unwrap();
        expected
            .execute_batch(V36_ACTION_DECISION_TRIGGERED_REPLAY_AUTHORITY_SQL)
            .unwrap();
        expected
            .execute_batch(V37_DECISION_H2A_SUPERSEDE_REPLAY_AUTHORITY_SQL)
            .unwrap();
        expected
            .execute_batch(V38_ACTION_H2A_COMPLETE_EVIDENCE_SNAPSHOT_SQL)
            .unwrap();
        expected.execute_batch(V39_EVIDENCE_RELOCATION_SQL).unwrap();
        expected
            .execute_batch(V40_EVIDENCE_SUPERSESSION_SQL)
            .unwrap();
        expected
            .execute_batch(V41_MANAGED_PROJECTION_REBUILD_SQL)
            .unwrap();
        expected
            .execute_batch(V42_MANAGED_PROJECTION_H1_AUTO_SQL)
            .unwrap();
        expected
            .execute_batch(V43_EVIDENCE_OBSERVED_UNPINNED_SQL)
            .unwrap();
        expected
            .execute_batch(V44_EVIDENCE_FINGERPRINT_PIN_SQL)
            .unwrap();
        expected
            .execute_batch(V45_WORK_MANAGEMENT_REJECTION_AND_EVIDENCE_BINDING_SQL)
            .unwrap();
        expected
            .execute_batch(V46_RISK_ISSUE_PREPARED_REJECTION_SQL)
            .unwrap();
        expected
            .execute_batch(V47_RECORD_ENTRY_FOUNDATION_SQL)
            .unwrap();
        expected.execute_batch(V48_EVIDENCE_FROM_FILE_SQL).unwrap();
        assert_eq!(
            schema_objects(&connection).unwrap(),
            schema_objects(&expected).unwrap()
        );
        let registry: Vec<(i64, String, String)> = connection
            .prepare(
                "SELECT version,migration_key,checksum FROM schema_migrations ORDER BY version",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(registry.len(), MIGRATIONS.len());
        assert_eq!(
            MIGRATIONS.len(),
            usize::try_from(CURRENT_SCHEMA_VERSION).unwrap()
        );
        assert_eq!(registry[3].0, 4);
        assert_eq!(registry[3].1, MIGRATIONS[3].key);
        assert_eq!(registry[3].2, MIGRATIONS[3].checksum);
        assert_eq!(registry[4].1, MIGRATIONS[4].key);
        assert_eq!(registry[4].2, MIGRATIONS[4].checksum);
        assert_eq!(registry[5].1, MIGRATIONS[5].key);
        assert_eq!(registry[5].2, MIGRATIONS[5].checksum);
        assert_eq!(registry[6].1, MIGRATIONS[6].key);
        assert_eq!(registry[6].2, MIGRATIONS[6].checksum);
        assert_eq!(registry[7].1, MIGRATIONS[7].key);
        assert_eq!(registry[7].2, MIGRATIONS[7].checksum);
        assert_eq!(registry[8].1, MIGRATIONS[8].key);
        assert_eq!(registry[8].2, MIGRATIONS[8].checksum);
        assert_eq!(registry[9].1, MIGRATIONS[9].key);
        assert_eq!(registry[9].2, MIGRATIONS[9].checksum);
        let metadata: (i64, i64, i64) = connection
            .query_row(
                "SELECT singleton,schema_version,ledger_revision FROM ledger_metadata",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(metadata, (1, i64::from(CURRENT_SCHEMA_VERSION), 7));
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            i64::from(CURRENT_SCHEMA_VERSION)
        );
        let sentinel: (String, String, String, String, String) = connection
            .query_row(
                "SELECT id,name,kind,provenance_kind,provenance_reference FROM stakeholders WHERE id='sentinel-owner'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(
            sentinel,
            (
                "sentinel-owner".to_owned(),
                "Synthetic owner".to_owned(),
                "person".to_owned(),
                "synthetic_fixture".to_owned(),
                "migration-test".to_owned(),
            )
        );
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn v12_rejects_a_terminal_denial_whose_audit_code_does_not_match_the_operation() {
        let mut connection = committed_v1_fixture();
        let transaction = connection.transaction().unwrap();
        migrate_v1_to_current_for_test(&transaction, false).unwrap();
        transaction.commit().unwrap();

        connection
            .execute_batch(
                "INSERT INTO aggregate_registry(id,aggregate_type,version,classification,created_at,updated_at) VALUES('synthetic-v12-risk','risk',1,'internal',0,0);\
                 INSERT INTO risks(id,title,details,state,response,owner_id,rationale,residual_exposure,next_review_at,in_exception_queue) VALUES('synthetic-v12-risk','Synthetic V12 risk','Public-safe migration fixture','open',NULL,NULL,NULL,NULL,NULL,1);\
                 INSERT INTO prepared_intents(id,contract_version,intent_kind,payload_digest,classification,policy,cancellation_policy,authority,expires_at,created_at) VALUES('synthetic-v12-prepared',1,'record_risk_occurrence','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','internal','allowed','not_cancellable_after_submit','head_of_products',100,0);\
                 INSERT INTO prepared_intent_targets(prepared_intent_id,ordinal,target_type,target_id,expected_version) VALUES('synthetic-v12-prepared',0,'risk','synthetic-v12-risk',1);\
                 INSERT INTO audit_events(id,occurred_at,actor,module,event_code,target_type,target_id,correlation_id,policy_outcome,approval_outcome,execution_outcome,effect_scope) VALUES('synthetic-v12-audit',1,'head_of_products','work_management','risk.close_denied','risk','synthetic-v12-risk','synthetic-v12-correlation','denied','not_required','not_attempted','none');",
            )
            .unwrap();

        assert!(connection
            .execute(
                "INSERT INTO risk_h2a_v11_terminal_denials(execute_idempotency_id,prepared_intent_id,risk_id,risk_version,operation,acknowledged_digest,correlation_id,policy_denial_audit_id,error_code,error_message_key,error_retryable) VALUES('synthetic-v12-execute','synthetic-v12-prepared','synthetic-v12-risk',1,'record_occurrence','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','synthetic-v12-correlation','synthetic-v12-audit','SECURITY_POLICY_DENIED','risk.policy_denied',0)",
                [],
            )
            .is_err());
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::expect_used)]
    fn v1_to_current_test_migration_rolls_back_every_change_after_injected_failure() {
        let mut connection = committed_v1_fixture();
        let v1_objects = schema_objects(&connection).unwrap();
        let v1_registry: Vec<(i64, String, String)> = connection
            .prepare(
                "SELECT version,migration_key,checksum FROM schema_migrations ORDER BY version",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let v1_metadata: (i64, i64, i64) = connection
            .query_row(
                "SELECT singleton,schema_version,ledger_revision FROM ledger_metadata",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        let transaction = connection
            .transaction()
            .unwrap_or_else(|_| panic!("synthetic v2 transaction"));
        assert_eq!(
            migrate_v1_to_current_for_test(&transaction, true),
            Err(LedgerOpenError::StorageUnavailable)
        );
        drop(transaction);
        assert_eq!(schema_objects(&connection).unwrap(), v1_objects);
        let registry: Vec<(i64, String, String)> = connection
            .prepare(
                "SELECT version,migration_key,checksum FROM schema_migrations ORDER BY version",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(registry, v1_registry);
        let metadata: (i64, i64, i64) = connection
            .query_row(
                "SELECT singleton,schema_version,ledger_revision FROM ledger_metadata",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(metadata, v1_metadata);
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        let sentinel: (String, String, String, String, String) = connection
            .query_row(
                "SELECT id,name,kind,provenance_kind,provenance_reference FROM stakeholders WHERE id='sentinel-owner'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(
            sentinel,
            (
                "sentinel-owner".to_owned(),
                "Synthetic owner".to_owned(),
                "person".to_owned(),
                "synthetic_fixture".to_owned(),
                "migration-test".to_owned(),
            )
        );
    }
}
