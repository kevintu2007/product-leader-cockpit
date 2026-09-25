use pmc_domain::classification::DataClassification;
use pmc_domain::classification_governance::{
    classify_external_ai_eligibility, resolve_inheritance, AuthoritativeClassificationState,
    ClassificationAuthorityPort, ClassificationAuthorityService, ClassificationDenialReason,
    ClassificationEligibility, ClassificationPortError, ClassificationSourceId,
    OrdinaryWriteContext, RequiredClassificationSources,
};
use std::collections::BTreeMap;

#[derive(Clone)]
struct FakeAuthority {
    manifest: Result<Vec<ClassificationSourceId>, ClassificationPortError>,
    states: BTreeMap<ClassificationSourceId, AuthoritativeClassificationState>,
}

impl ClassificationAuthorityPort for FakeAuthority {
    type Subject = String;

    fn required_source_manifest(
        &self,
        _subject: &Self::Subject,
    ) -> Result<Vec<ClassificationSourceId>, ClassificationPortError> {
        self.manifest.clone()
    }

    fn read_classification(
        &self,
        _subject: &Self::Subject,
        source_id: ClassificationSourceId,
    ) -> Result<AuthoritativeClassificationState, ClassificationPortError> {
        Ok(self
            .states
            .get(&source_id)
            .copied()
            .unwrap_or(AuthoritativeClassificationState::Missing))
    }
}

fn id(value: u64) -> ClassificationSourceId {
    ClassificationSourceId::from_stable_key(value)
}

fn subject() -> String {
    "synthetic-subject".to_owned()
}

#[test]
fn public_snapshot_can_only_expose_fail_closed_unavailable_state() {
    let sources = RequiredClassificationSources::unavailable();
    let resolved = resolve_inheritance(&sources);
    assert_eq!(resolved.classification(), DataClassification::Unclassified);
    assert!(resolved.fail_closed());
    assert_eq!(resolved.trusted_source_count(), 0);
}

#[test]
fn public_eligibility_denies_unclassified_without_payload_or_dispatch() {
    let result = classify_external_ai_eligibility(&RequiredClassificationSources::unavailable());
    assert!(matches!(
        result,
        ClassificationEligibility::Denied {
            classification: DataClassification::Unclassified,
            reason: ClassificationDenialReason::Unclassified,
        }
    ));
}

#[test]
fn public_ordinary_context_has_no_quarantine_capability() {
    assert!(!OrdinaryWriteContext::ordinary().allows_quarantine());
}

#[test]
fn authority_service_resolves_four_sources_and_ordinary_write_consumes_result() {
    let source_ids = vec![id(1), id(2), id(3), id(4)];
    let states = BTreeMap::from([
        (
            id(1),
            AuthoritativeClassificationState::Classified(DataClassification::Public),
        ),
        (
            id(2),
            AuthoritativeClassificationState::Classified(DataClassification::Internal),
        ),
        (
            id(3),
            AuthoritativeClassificationState::Classified(DataClassification::Public),
        ),
        (
            id(4),
            AuthoritativeClassificationState::Classified(DataClassification::Restricted),
        ),
    ]);
    let service = ClassificationAuthorityService::new(FakeAuthority {
        manifest: Ok(source_ids),
        states,
    });
    let resolved = service.resolve(&subject());
    assert_eq!(resolved.classification(), DataClassification::Restricted);
    assert!(!resolved.fail_closed());
    assert!(matches!(
        OrdinaryWriteContext::ordinary().decide(DataClassification::Internal, &resolved),
        pmc_domain::classification_governance::OrdinaryWriteDecision::Applied {
            classification: DataClassification::Restricted
        }
    ));
}

#[test]
fn authority_service_fails_closed_for_missing_fourth_duplicate_and_unavailable_manifest() {
    let missing_fourth = ClassificationAuthorityService::new(FakeAuthority {
        manifest: Ok(vec![id(1), id(2), id(3), id(4)]),
        states: BTreeMap::from([
            (
                id(1),
                AuthoritativeClassificationState::Classified(DataClassification::Public),
            ),
            (
                id(2),
                AuthoritativeClassificationState::Classified(DataClassification::Public),
            ),
            (
                id(3),
                AuthoritativeClassificationState::Classified(DataClassification::Public),
            ),
        ]),
    });
    assert!(missing_fourth.resolve(&subject()).fail_closed());

    let duplicate = ClassificationAuthorityService::new(FakeAuthority {
        manifest: Ok(vec![id(1), id(1)]),
        states: BTreeMap::new(),
    });
    assert!(duplicate.resolve(&subject()).fail_closed());

    let unavailable = ClassificationAuthorityService::new(FakeAuthority {
        manifest: Err(ClassificationPortError::Unavailable),
        states: BTreeMap::new(),
    });
    assert!(unavailable.resolve(&subject()).fail_closed());
}

#[test]
fn authority_service_reaches_public_and_internal_classification_eligibility() {
    for (classification, expected) in [
        (
            DataClassification::Public,
            ClassificationEligibility::RequiresExternalAiGates {
                classification: DataClassification::Public,
            },
        ),
        (
            DataClassification::Internal,
            ClassificationEligibility::RequiresExplicitOrganizationPolicy {
                classification: DataClassification::Internal,
            },
        ),
    ] {
        let service = ClassificationAuthorityService::new(FakeAuthority {
            manifest: Ok(vec![id(1)]),
            states: BTreeMap::from([(
                id(1),
                AuthoritativeClassificationState::Classified(classification),
            )]),
        });
        assert_eq!(service.external_ai_eligibility(&subject()), expected);
    }
}
