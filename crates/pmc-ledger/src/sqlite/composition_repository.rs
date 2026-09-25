//! The SQLite read surface the Cockpit, People and Product-health route
//! compositions need.
//!
//! Reads Stakeholders, their responsibility/dependency relationships, and
//! Milestones. These are the facts the People routes and the Product-health
//! inspector need and that the projection snapshot deliberately does not
//! carry.
//!
//! Everything is read inside **one transaction**, so the collections cannot
//! disagree about which instant they describe, and each is ordered by its own
//! stable identifier so repeated reads of unchanged state compare equal.
//!
//! Version and classification come from `aggregate_registry` rather than from
//! the record tables, because that is where the Ledger keeps them -- a
//! composition that invented either would be asserting authority it does not
//! have.

use pmc_domain::classification::DataClassification;
use pmc_domain::composition_source::{
    ActionRequestReadRecord, CompositionSnapshotReadError, DecisionRequestReadRecord,
    EvidenceLinkReadRecord, EvidenceReferenceReadRecord, InitiativeReadRecord, IssueReadRecord,
    KpiDefinitionReadRecord, KpiObservationReadRecord, LedgerCompositionSnapshot,
    LedgerSnapshotForCompositionPort, MilestoneReadRecord, PortfolioRelationshipReadRecord,
    ProductReadRecord, ProjectReadRecord, RelationshipEndpointReadRecord, RiskReadRecord,
    RoadmapReadRecord, StakeholderReadRecord, StakeholderRelationshipReadRecord,
    WorkOwnerReadRecord,
};
use pmc_domain::identity::{
    AggregateVersion, EvidenceReferenceId, InitiativeId, KpiId, KpiObservationId, MilestoneId,
    ProductId, ProjectId, RoadmapId, StakeholderId,
};
use pmc_domain::relationships::{
    RelationshipKind, StakeholderKind, StakeholderRelationshipPurpose,
};
use pmc_domain::time::UtcTimestamp;
use pmc_domain::work_management::{
    ActionRequestState, DecisionRequestState, EvidenceRole, EvidenceVerification, IntegrityDigest,
    IssueState,
};
use rusqlite::Transaction;

use super::{SqliteProductLedger, CURRENT_SCHEMA_VERSION};

fn read_failed<T>(_: T) -> CompositionSnapshotReadError {
    CompositionSnapshotReadError::ReadFailed
}

fn decode_version(raw: i64) -> Result<AggregateVersion, CompositionSnapshotReadError> {
    u64::try_from(raw)
        .ok()
        .and_then(|value| AggregateVersion::new(value).ok())
        .ok_or(CompositionSnapshotReadError::ReadFailed)
}

fn decode_classification(raw: &str) -> Result<DataClassification, CompositionSnapshotReadError> {
    DataClassification::from_persisted(raw).map_err(read_failed)
}

fn read_stakeholders(
    tx: &Transaction<'_>,
) -> Result<Vec<StakeholderReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT s.id,s.name,s.kind,r.version,r.classification \
             FROM stakeholders s \
             JOIN aggregate_registry r ON r.id=s.id AND r.aggregate_type='stakeholder' \
             ORDER BY s.id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (id, name, kind, version, classification) = row.map_err(read_failed)?;
        records.push(StakeholderReadRecord {
            id: StakeholderId::parse(id).map_err(read_failed)?,
            display_name: name,
            kind: StakeholderKind::from_persisted(&kind).map_err(read_failed)?,
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
        });
    }
    Ok(records)
}

/// Reads stakeholder-subject relationships by joining each relationship to
/// both of its endpoints.
///
/// A `stakeholder_subject` relationship has exactly two endpoints: the
/// Stakeholder and the thing they are responsible for or depend on. The join
/// requires the stakeholder endpoint to be the one whose target is a
/// stakeholder, so a relationship whose endpoints are malformed produces no
/// row rather than a guess about which end is which.
fn read_stakeholder_relationships(
    tx: &Transaction<'_>,
) -> Result<Vec<StakeholderRelationshipReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT holder.target_id,subject.target_type,subject.target_id,rel.purpose,\
                    reg.version,reg.classification \
             FROM relationships rel \
             JOIN relationship_endpoints holder \
                  ON holder.relationship_id=rel.id AND holder.target_type='stakeholder' \
             JOIN relationship_endpoints subject \
                  ON subject.relationship_id=rel.id AND subject.target_type<>'stakeholder' \
             JOIN aggregate_registry reg ON reg.id=rel.id AND reg.aggregate_type='relationship' \
             WHERE rel.kind='stakeholder_subject' AND rel.purpose IS NOT NULL \
             ORDER BY holder.target_id,subject.target_id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (holder, subject_type, subject_id, purpose, version, classification) =
            row.map_err(read_failed)?;
        records.push(StakeholderRelationshipReadRecord {
            stakeholder_id: StakeholderId::parse(holder).map_err(read_failed)?,
            subject_type,
            subject_id,
            purpose: StakeholderRelationshipPurpose::from_persisted(&purpose)
                .map_err(read_failed)?,
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
        });
    }
    Ok(records)
}

fn read_milestones(
    tx: &Transaction<'_>,
) -> Result<Vec<MilestoneReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT m.id,m.project_id,m.name,m.due_at,r.version,r.classification \
             FROM milestones m \
             JOIN aggregate_registry r ON r.id=m.id AND r.aggregate_type='milestone' \
             ORDER BY m.id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (id, project_id, name, due_at, version, classification) = row.map_err(read_failed)?;
        records.push(MilestoneReadRecord {
            id: MilestoneId::parse(id).map_err(read_failed)?,
            project_id: ProjectId::parse(project_id).map_err(read_failed)?,
            name,
            due_at: UtcTimestamp::from_unix_millis(due_at),
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
        });
    }
    Ok(records)
}

fn read_action_requests(
    tx: &Transaction<'_>,
) -> Result<Vec<ActionRequestReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT a.id,a.title,a.intended_owner_id,a.state,a.response_due_at,a.intended_action_due_at,\
                    r.version,r.classification \
             FROM action_requests a \
             JOIN aggregate_registry r ON r.id=a.id AND r.aggregate_type='action_request' \
             ORDER BY a.id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (
            id,
            title,
            owner,
            state,
            response_due_at,
            intended_action_due_at,
            version,
            classification,
        ) = row.map_err(read_failed)?;
        records.push(ActionRequestReadRecord {
            id,
            title,
            // A missing owner survives the read rather than being dropped:
            // its absence is the fact behind the missing-owner attention
            // reason.
            intended_owner_id: owner
                .map(StakeholderId::parse)
                .transpose()
                .map_err(read_failed)?,
            state: ActionRequestState::from_persisted(&state).map_err(read_failed)?,
            response_due_at: response_due_at.map(UtcTimestamp::from_unix_millis),
            intended_action_due_at: intended_action_due_at.map(UtcTimestamp::from_unix_millis),
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
        });
    }
    Ok(records)
}

fn read_decision_requests(
    tx: &Transaction<'_>,
) -> Result<Vec<DecisionRequestReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT d.id,d.subject,d.intended_owner_id,d.state,r.version,r.classification \
             FROM decision_requests d \
             JOIN aggregate_registry r ON r.id=d.id AND r.aggregate_type='decision_request' \
             ORDER BY d.id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (id, subject, owner, state, version, classification) = row.map_err(read_failed)?;
        records.push(DecisionRequestReadRecord {
            id,
            subject,
            intended_owner_id: owner
                .map(StakeholderId::parse)
                .transpose()
                .map_err(read_failed)?,
            state: DecisionRequestState::from_persisted(&state).map_err(read_failed)?,
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
        });
    }
    Ok(records)
}

fn read_issues(tx: &Transaction<'_>) -> Result<Vec<IssueReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT i.id,i.title,i.state,i.source_risk_id,i.recurrence_of_id,\
                    r.version,r.classification \
             FROM issues i \
             JOIN aggregate_registry r ON r.id=i.id AND r.aggregate_type='issue' \
             ORDER BY i.id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (id, title, state, source_risk_id, recurrence_of_id, version, classification) =
            row.map_err(read_failed)?;
        records.push(IssueReadRecord {
            id,
            title,
            state: IssueState::from_persisted(&state).map_err(read_failed)?,
            // Both origins survive the read: an Issue from a Risk and a
            // recurrence of an earlier Issue are materially different from a
            // fresh one, and `IssueRecurrence` attention exists because of
            // the second.
            source_risk_id,
            recurrence_of_id,
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
        });
    }
    Ok(records)
}

fn read_risks(tx: &Transaction<'_>) -> Result<Vec<RiskReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT k.id,k.title,r.version,r.classification \
             FROM risks k \
             JOIN aggregate_registry r ON r.id=k.id AND r.aggregate_type='risk' \
             ORDER BY k.id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (id, title, version, classification) = row.map_err(read_failed)?;
        records.push(RiskReadRecord {
            id,
            title,
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
        });
    }
    Ok(records)
}

/// Reads every Portfolio-hierarchy relationship: the six kinds other than
/// `stakeholder_subject`.
///
/// The self-join `a.target_type < b.target_type` yields exactly one row per
/// well-formed relationship and orders its two endpoints by type, which is
/// all a caller needs because every Portfolio kind pairs two different
/// types. A relationship whose endpoints are malformed -- fewer than two, or
/// three or more -- yields zero or several rows for one `rel.id`; both are
/// dropped rather than one pair being picked, matching how the Stakeholder
/// read treats malformed rows. Picking a pair would attribute a relationship
/// the Ledger does not hold, which is worse than omitting one it does.
fn read_portfolio_relationships(
    tx: &Transaction<'_>,
) -> Result<Vec<PortfolioRelationshipReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT rel.id,rel.kind,a.target_type,a.target_id,b.target_type,b.target_id,\
                    reg.version,reg.classification \
             FROM relationships rel \
             JOIN relationship_endpoints a ON a.relationship_id=rel.id \
             JOIN relationship_endpoints b \
                  ON b.relationship_id=rel.id AND a.target_type<b.target_type \
             JOIN aggregate_registry reg ON reg.id=rel.id AND reg.aggregate_type='relationship' \
             WHERE rel.kind<>'stakeholder_subject' \
             ORDER BY rel.id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records: Vec<PortfolioRelationshipReadRecord> = Vec::new();
    let mut malformed: Vec<String> = Vec::new();
    for row in rows {
        let (id, kind, a_type, a_id, b_type, b_id, version, classification) =
            row.map_err(read_failed)?;
        if records.last().is_some_and(|last| last.id == id) {
            // A second row for the same relationship means it has more than
            // two endpoints. Remember it so every row of it is dropped.
            malformed.push(id);
            continue;
        }
        records.push(PortfolioRelationshipReadRecord {
            id,
            kind: RelationshipKind::from_persisted(&kind).map_err(read_failed)?,
            endpoints: [
                RelationshipEndpointReadRecord {
                    target_type: a_type,
                    target_id: a_id,
                },
                RelationshipEndpointReadRecord {
                    target_type: b_type,
                    target_id: b_id,
                },
            ],
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
        });
    }
    records.retain(|record| !malformed.contains(&record.id));
    Ok(records)
}

fn read_products(
    tx: &Transaction<'_>,
) -> Result<Vec<ProductReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT p.id,p.name,r.version,r.classification \
             FROM products p \
             JOIN aggregate_registry r ON r.id=p.id AND r.aggregate_type='product' \
             ORDER BY p.id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (id, name, version, classification) = row.map_err(read_failed)?;
        records.push(ProductReadRecord {
            id: ProductId::parse(id).map_err(read_failed)?,
            name,
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
        });
    }
    Ok(records)
}

fn read_initiatives(
    tx: &Transaction<'_>,
) -> Result<Vec<InitiativeReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT i.id,i.name,r.version,r.classification \
             FROM initiatives i \
             JOIN aggregate_registry r ON r.id=i.id AND r.aggregate_type='initiative' \
             ORDER BY i.id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (id, name, version, classification) = row.map_err(read_failed)?;
        records.push(InitiativeReadRecord {
            id: InitiativeId::parse(id).map_err(read_failed)?,
            name,
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
        });
    }
    Ok(records)
}

fn read_projects(
    tx: &Transaction<'_>,
) -> Result<Vec<ProjectReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT p.id,p.name,p.start_at,p.end_at,r.version,r.classification \
             FROM projects p \
             JOIN aggregate_registry r ON r.id=p.id AND r.aggregate_type='project' \
             ORDER BY p.id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (id, name, start_at, end_at, version, classification) = row.map_err(read_failed)?;
        records.push(ProjectReadRecord {
            id: ProjectId::parse(id).map_err(read_failed)?,
            name,
            start_at: UtcTimestamp::from_unix_millis(start_at),
            end_at: UtcTimestamp::from_unix_millis(end_at),
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
        });
    }
    Ok(records)
}

fn read_roadmaps(
    tx: &Transaction<'_>,
) -> Result<Vec<RoadmapReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT m.id,m.name,r.version,r.classification \
             FROM roadmaps m \
             JOIN aggregate_registry r ON r.id=m.id AND r.aggregate_type='roadmap' \
             ORDER BY m.id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (id, name, version, classification) = row.map_err(read_failed)?;
        records.push(RoadmapReadRecord {
            id: RoadmapId::parse(id).map_err(read_failed)?,
            name,
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
        });
    }
    Ok(records)
}

fn read_kpi_definitions(
    tx: &Transaction<'_>,
) -> Result<Vec<KpiDefinitionReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT k.id,k.name,r.version,r.classification \
             FROM kpi_definitions k \
             JOIN aggregate_registry r ON r.id=k.id AND r.aggregate_type='kpi_definition' \
             ORDER BY k.id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (id, name, version, classification) = row.map_err(read_failed)?;
        records.push(KpiDefinitionReadRecord {
            id: KpiId::parse(id).map_err(read_failed)?,
            name,
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
        });
    }
    Ok(records)
}

/// Reads that each KPI observation exists and when it was made. `value` and
/// `source` are not selected: a measured value never enters this read
/// surface (DG3 S01 amendment 2026-09-15, §8.2).
fn read_kpi_observations(
    tx: &Transaction<'_>,
) -> Result<Vec<KpiObservationReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT o.id,o.kpi_id,o.observed_at,r.version,r.classification \
             FROM kpi_observations o \
             JOIN aggregate_registry r ON r.id=o.id AND r.aggregate_type='kpi_observation' \
             ORDER BY o.id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (id, kpi_id, observed_at, version, classification) = row.map_err(read_failed)?;
        records.push(KpiObservationReadRecord {
            id: KpiObservationId::parse(id).map_err(read_failed)?,
            kpi_id: KpiId::parse(kpi_id).map_err(read_failed)?,
            observed_at: UtcTimestamp::from_unix_millis(observed_at),
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
        });
    }
    Ok(records)
}

/// Reads every Evidence link. The link's own classification column is the
/// one recorded at link time and is surfaced under that name.
fn read_evidence_links(
    tx: &Transaction<'_>,
) -> Result<Vec<EvidenceLinkReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT evidence_id,target_type,target_id,classification,linked_at \
             FROM evidence_links \
             ORDER BY evidence_id,target_type,target_id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (evidence_id, target_type, target_id, classification, linked_at) =
            row.map_err(read_failed)?;
        records.push(EvidenceLinkReadRecord {
            evidence_id: EvidenceReferenceId::parse(evidence_id).map_err(read_failed)?,
            target_type,
            target_id,
            classification_at_link: decode_classification(&classification)?,
            linked_at: UtcTimestamp::from_unix_millis(linked_at),
        });
    }
    Ok(records)
}

/// Reads every Evidence reference's current state. The Vault path is not
/// selected: it never enters a read surface.
fn read_evidence_references(
    tx: &Transaction<'_>,
) -> Result<Vec<EvidenceReferenceReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT er.id,er.role,er.verification,er.last_verified_at,er.integrity_digest,\
                    r.version,r.classification,r.updated_at,\
                    er.fingerprint_algorithm IS NOT NULL \
             FROM evidence_references er \
             JOIN aggregate_registry r ON r.id=er.id AND r.aggregate_type='evidence_reference' \
             ORDER BY er.id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, bool>(8)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (
            id,
            role,
            verification,
            last_verified_at,
            integrity_digest,
            version,
            classification,
            updated_at,
            pinned,
        ) = row.map_err(read_failed)?;
        let digest = integrity_digest
            .map(IntegrityDigest::parse)
            .transpose()
            .map_err(read_failed)?;
        records.push(EvidenceReferenceReadRecord {
            id: EvidenceReferenceId::parse(id).map_err(read_failed)?,
            role: role
                .as_deref()
                .map(EvidenceRole::from_persisted)
                .transpose()
                .map_err(read_failed)?,
            verification: EvidenceVerification::from_persisted_parts(
                &verification,
                last_verified_at.map(UtcTimestamp::from_unix_millis),
                digest,
            )
            .map_err(read_failed)?,
            pinned,
            classification: decode_classification(&classification)?,
            version: decode_version(version)?,
            updated_at: UtcTimestamp::from_unix_millis(updated_at),
        });
    }
    Ok(records)
}

/// Reads who owns every owned work record, across the five tables that
/// carry an owner, as one ordered collection.
///
/// Rows whose owner column is NULL (an unowned Risk, a request with no
/// intended owner) are excluded here rather than read as a blank owner: an
/// absent owner is a fact the attention evaluator reports, not a person.
fn read_work_owners(
    tx: &Transaction<'_>,
) -> Result<Vec<WorkOwnerReadRecord>, CompositionSnapshotReadError> {
    let mut statement = tx
        .prepare(
            "SELECT target_type,target_id,owner_id FROM (                SELECT 'action_request' AS target_type,id AS target_id,intended_owner_id AS owner_id                   FROM action_requests WHERE intended_owner_id IS NOT NULL                 UNION ALL                 SELECT 'action',id,owner_id FROM actions                 UNION ALL                 SELECT 'decision_request',id,intended_owner_id                   FROM decision_requests WHERE intended_owner_id IS NOT NULL                 UNION ALL                 SELECT 'decision',id,owner_id FROM decisions                 UNION ALL                 SELECT 'risk',id,owner_id FROM risks WHERE owner_id IS NOT NULL             ) ORDER BY target_type,target_id",
        )
        .map_err(read_failed)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(read_failed)?;
    let mut records = Vec::new();
    for row in rows {
        let (target_type, target_id, owner_id) = row.map_err(read_failed)?;
        records.push(WorkOwnerReadRecord {
            target_type,
            target_id,
            owner_id: StakeholderId::parse(owner_id).map_err(read_failed)?,
        });
    }
    Ok(records)
}

impl LedgerSnapshotForCompositionPort for SqliteProductLedger {
    fn read_composition_snapshot(
        &self,
        observed_at: UtcTimestamp,
    ) -> Result<LedgerCompositionSnapshot, CompositionSnapshotReadError> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(CompositionSnapshotReadError::Unavailable);
        }
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| CompositionSnapshotReadError::Unavailable)?;
        let ledger_revision: i64 = transaction
            .query_row(
                "SELECT ledger_revision FROM ledger_metadata WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(read_failed)?;
        let snapshot = LedgerCompositionSnapshot {
            schema_version: self.schema_version,
            ledger_revision: u64::try_from(ledger_revision).map_err(read_failed)?,
            ledger_as_of_utc: observed_at,
            stakeholders: read_stakeholders(&transaction)?,
            stakeholder_relationships: read_stakeholder_relationships(&transaction)?,
            milestones: read_milestones(&transaction)?,
            action_requests: read_action_requests(&transaction)?,
            decision_requests: read_decision_requests(&transaction)?,
            issues: read_issues(&transaction)?,
            risks: read_risks(&transaction)?,
            portfolio_relationships: read_portfolio_relationships(&transaction)?,
            products: read_products(&transaction)?,
            initiatives: read_initiatives(&transaction)?,
            projects: read_projects(&transaction)?,
            roadmaps: read_roadmaps(&transaction)?,
            kpi_definitions: read_kpi_definitions(&transaction)?,
            kpi_observations: read_kpi_observations(&transaction)?,
            evidence_links: read_evidence_links(&transaction)?,
            evidence_references: read_evidence_references(&transaction)?,
            work_owners: read_work_owners(&transaction)?,
        };
        transaction.finish().map_err(read_failed)?;
        Ok(snapshot)
    }
}
