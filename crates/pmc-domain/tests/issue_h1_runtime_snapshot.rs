use pmc_domain::classification::DataClassification;
use pmc_domain::identity::{AggregateVersion, IssueId, RiskId};
use pmc_domain::issues::{IssueH1RuntimeSnapshot, IssueRecord};
use pmc_domain::BoundedText;

fn text<const N: usize>(value: &str) -> BoundedText<N> {
    BoundedText::parse(value.to_owned()).unwrap()
}

fn independent_issue(id: &str) -> IssueRecord {
    IssueRecord::from_persisted_created_open(
        IssueId::parse(id).unwrap(),
        text("Synthetic issue"),
        text("Public-safe restart fixture"),
        DataClassification::Internal,
        AggregateVersion::initial(),
    )
    .unwrap()
}

#[test]
fn accepts_independent_initial_open_records() {
    let snapshot = IssueH1RuntimeSnapshot::try_new(vec![independent_issue("issue-alpha")])
        .expect("canonical independent Issue H1 record is accepted");

    assert_eq!(snapshot.len(), 1);
}

#[test]
fn rejects_duplicate_issue_identity() {
    let issue = independent_issue("issue-duplicate");

    assert!(matches!(
        IssueH1RuntimeSnapshot::try_new(vec![issue.clone(), issue]),
        Err(pmc_domain::issues::IssueH1RuntimeSnapshotError::DuplicateIssue)
    ));
}

#[test]
fn rejects_risk_derived_issue_records() {
    let issue = IssueRecord::from_persisted_created_from_risk(
        IssueId::parse("issue-from-risk").unwrap(),
        RiskId::parse("risk-source").unwrap(),
        text("Synthetic issue"),
        text("Public-safe restart fixture"),
        DataClassification::Internal,
        AggregateVersion::initial(),
    )
    .unwrap();

    assert!(matches!(
        IssueH1RuntimeSnapshot::try_new(vec![issue]),
        Err(pmc_domain::issues::IssueH1RuntimeSnapshotError::InvalidRecord)
    ));
}

#[test]
fn rejects_unclassified_independent_records_at_decode_boundary() {
    assert!(IssueRecord::from_persisted_created_open(
        IssueId::parse("issue-unclassified").unwrap(),
        text("Synthetic issue"),
        text("Public-safe restart fixture"),
        DataClassification::Unclassified,
        AggregateVersion::initial(),
    )
    .is_err());
}
