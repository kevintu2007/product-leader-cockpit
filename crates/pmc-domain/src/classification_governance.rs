//! Fail-closed classification authority and eligibility seams.
//!
//! A domain-owned authority service reads a complete source manifest and each
//! current source state through a narrow port, then mints an opaque snapshot.
//! This module never authorizes a provider or creates a payload.

use crate::classification::DataClassification;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ClassificationSourceId(u64);

impl ClassificationSourceId {
    #[must_use]
    pub const fn from_stable_key(value: u64) -> Self {
        Self(value)
    }
}

pub(crate) use ClassificationSourceId as RequiredSourceId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthoritativeClassificationState {
    Classified(DataClassification),
    Missing,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClassificationPortError {
    Unavailable,
    ReadFailed,
}

pub trait ClassificationAuthorityPort {
    type Subject: Clone;

    fn required_source_manifest(
        &self,
        subject: &Self::Subject,
    ) -> Result<Vec<ClassificationSourceId>, ClassificationPortError>;

    fn read_classification(
        &self,
        subject: &Self::Subject,
        source_id: ClassificationSourceId,
    ) -> Result<AuthoritativeClassificationState, ClassificationPortError>;
}

pub struct ClassificationAuthorityService<P> {
    port: P,
}

impl<P> ClassificationAuthorityService<P>
where
    P: ClassificationAuthorityPort,
{
    #[must_use]
    pub const fn new(port: P) -> Self {
        Self { port }
    }

    #[must_use]
    pub fn resolve(&self, subject: &P::Subject) -> ResolvedClassification {
        let Ok(manifest) = self.port.required_source_manifest(subject) else {
            return resolve_inheritance(&RequiredClassificationSources::unavailable());
        };
        if manifest.is_empty() || has_duplicate_ids(&manifest) {
            return resolve_inheritance(&RequiredClassificationSources::unavailable());
        }
        let entries = manifest
            .iter()
            .map(
                |source_id| match self.port.read_classification(subject, *source_id) {
                    Ok(AuthoritativeClassificationState::Classified(value)) => {
                        ClassificationSourceState::Trusted(TrustedClassificationSource::new(
                            *source_id, value,
                        ))
                    }
                    Ok(AuthoritativeClassificationState::Missing) => {
                        ClassificationSourceState::Missing(*source_id)
                    }
                    Ok(AuthoritativeClassificationState::Unknown) => {
                        ClassificationSourceState::Unknown(*source_id)
                    }
                    Err(_) => ClassificationSourceState::Unknown(*source_id),
                },
            )
            .collect();
        let snapshot = match RequiredClassificationSources::from_authority(manifest, entries) {
            Ok(snapshot) => snapshot,
            Err(_) => RequiredClassificationSources::unavailable(),
        };
        resolve_inheritance(&snapshot)
    }

    #[must_use]
    pub fn external_ai_eligibility(&self, subject: &P::Subject) -> ClassificationEligibility {
        let resolved = self.resolve(subject);
        eligibility_for_resolved(&resolved)
    }
}

fn has_duplicate_ids(ids: &[ClassificationSourceId]) -> bool {
    ids.iter()
        .enumerate()
        .any(|(index, id)| ids[..index].contains(id))
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TrustedClassificationSource {
    id: RequiredSourceId,
    classification: DataClassification,
}

#[allow(dead_code)]
impl TrustedClassificationSource {
    pub(crate) const fn new(id: RequiredSourceId, classification: DataClassification) -> Self {
        Self { id, classification }
    }
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ClassificationSourceState {
    Trusted(TrustedClassificationSource),
    AiSuggestion(RequiredSourceId, DataClassification),
    Missing(RequiredSourceId),
    Unknown(RequiredSourceId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SnapshotValidationError {
    EmptyManifest,
    CountMismatch,
    DuplicateManifestId,
    MissingManifestId,
    UnexpectedEntryId,
    DuplicateEntryId,
}

impl ClassificationSourceState {
    fn id(self) -> Option<RequiredSourceId> {
        match self {
            Self::Trusted(value) => Some(value.id),
            Self::AiSuggestion(id, _) | Self::Missing(id) | Self::Unknown(id) => Some(id),
        }
    }
}

/// An authority-produced complete source snapshot. Its fields and constructor
/// are private to the domain crate, so external callers cannot mint trust or
/// omit a required source identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequiredClassificationSources {
    manifest: Vec<RequiredSourceId>,
    entries: Vec<ClassificationSourceState>,
}

impl RequiredClassificationSources {
    /// A safe public value for a new/unavailable record: it is intentionally
    /// incomplete and therefore always resolves to Unclassified.
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            manifest: vec![ClassificationSourceId(0)],
            entries: vec![ClassificationSourceState::Missing(ClassificationSourceId(
                0,
            ))],
        }
    }

    #[allow(dead_code)]
    pub(crate) fn from_authority(
        manifest: Vec<RequiredSourceId>,
        entries: Vec<ClassificationSourceState>,
    ) -> Result<Self, SnapshotValidationError> {
        if manifest.is_empty() {
            return Err(SnapshotValidationError::EmptyManifest);
        }
        if manifest.len() != entries.len() {
            return Err(SnapshotValidationError::CountMismatch);
        }
        let entry_ids: Vec<RequiredSourceId> = entries
            .iter()
            .copied()
            .filter_map(ClassificationSourceState::id)
            .collect();
        for (index, id) in manifest.iter().enumerate() {
            if manifest[..index].contains(id) {
                return Err(SnapshotValidationError::DuplicateManifestId);
            }
            if !entry_ids.contains(id) {
                return Err(SnapshotValidationError::MissingManifestId);
            }
        }
        for id in &entry_ids {
            if !manifest.contains(id) {
                return Err(SnapshotValidationError::UnexpectedEntryId);
            }
            if entry_ids
                .iter()
                .filter(|candidate| *candidate == id)
                .count()
                > 1
            {
                return Err(SnapshotValidationError::DuplicateEntryId);
            }
        }
        Ok(Self { manifest, entries })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedClassification {
    classification: DataClassification,
    fail_closed: bool,
    trusted_source_count: usize,
}

impl ResolvedClassification {
    #[must_use]
    pub const fn classification(&self) -> DataClassification {
        self.classification
    }

    #[must_use]
    pub const fn fail_closed(&self) -> bool {
        self.fail_closed
    }

    #[must_use]
    pub const fn trusted_source_count(&self) -> usize {
        self.trusted_source_count
    }
}

#[must_use]
pub fn resolve_inheritance(sources: &RequiredClassificationSources) -> ResolvedClassification {
    let mut classification = DataClassification::Public;
    let mut trusted_source_count = 0;
    let mut fail_closed = false;

    for required_id in &sources.manifest {
        let matching: Vec<ClassificationSourceState> = sources
            .entries
            .iter()
            .copied()
            .filter(|entry| entry.id() == Some(*required_id))
            .collect();
        if matching.len() != 1 {
            fail_closed = true;
            continue;
        }
        match matching[0] {
            ClassificationSourceState::Trusted(trusted) => {
                trusted_source_count += 1;
                classification = classification.combine(trusted.classification);
                if trusted.classification == DataClassification::Unclassified {
                    fail_closed = true;
                }
            }
            ClassificationSourceState::AiSuggestion(_, _)
            | ClassificationSourceState::Missing(_)
            | ClassificationSourceState::Unknown(_) => {
                fail_closed = true;
            }
        }
    }

    if trusted_source_count == 0 || fail_closed {
        classification = DataClassification::Unclassified;
        fail_closed = true;
    }

    ResolvedClassification {
        classification,
        fail_closed,
        trusted_source_count,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrdinaryWriteContext {
    allow_quarantine: bool,
}

impl OrdinaryWriteContext {
    #[must_use]
    pub const fn ordinary() -> Self {
        Self {
            allow_quarantine: false,
        }
    }

    #[allow(dead_code)]
    pub(crate) const fn from_authority(allow_quarantine: bool) -> Self {
        Self { allow_quarantine }
    }

    #[must_use]
    pub const fn allows_quarantine(self) -> bool {
        self.allow_quarantine
    }

    #[must_use]
    pub fn decide(
        self,
        current: DataClassification,
        proposed: &ResolvedClassification,
    ) -> OrdinaryWriteDecision {
        let proposed = proposed.classification;
        if proposed == DataClassification::Unclassified {
            if self.allow_quarantine {
                return OrdinaryWriteDecision::Quarantined {
                    classification: proposed,
                };
            }
            if current == DataClassification::Unclassified {
                return OrdinaryWriteDecision::QuarantineNotAllowed;
            }
            return OrdinaryWriteDecision::RequiresH2aLowering(RequiresH2aLowering {
                current,
                proposed,
            });
        }

        let lowering =
            current != DataClassification::Unclassified && current.rank() > proposed.rank();
        if lowering {
            return OrdinaryWriteDecision::RequiresH2aLowering(RequiresH2aLowering {
                current,
                proposed,
            });
        }

        OrdinaryWriteDecision::Applied {
            classification: proposed,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrdinaryWriteDecision {
    Applied { classification: DataClassification },
    Quarantined { classification: DataClassification },
    QuarantineNotAllowed,
    RequiresH2aLowering(RequiresH2aLowering),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequiresH2aLowering {
    current: DataClassification,
    proposed: DataClassification,
}

impl RequiresH2aLowering {
    #[must_use]
    pub const fn current(self) -> DataClassification {
        self.current
    }

    #[must_use]
    pub const fn proposed(self) -> DataClassification {
        self.proposed
    }

    #[must_use]
    pub const fn requires_impact_preview(self) -> bool {
        true
    }

    #[must_use]
    pub const fn requires_rationale(self) -> bool {
        true
    }

    #[must_use]
    pub const fn requires_explicit_human_approval(self) -> bool {
        true
    }

    #[must_use]
    pub const fn requires_audit_event(self) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClassificationEligibility {
    RequiresExternalAiGates {
        classification: DataClassification,
    },
    RequiresExplicitOrganizationPolicy {
        classification: DataClassification,
    },
    Denied {
        classification: DataClassification,
        reason: ClassificationDenialReason,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClassificationDenialReason {
    Unclassified,
    Restricted,
    Confidential,
}

impl ClassificationEligibility {
    #[must_use]
    pub const fn classification(self) -> DataClassification {
        match self {
            Self::RequiresExternalAiGates { classification }
            | Self::RequiresExplicitOrganizationPolicy { classification }
            | Self::Denied { classification, .. } => classification,
        }
    }
}

/// Classification-only eligibility. Provider/account/purpose/retention/hash/
/// expiry/receipt gates belong to the later dedicated H2b seam.
#[must_use]
pub fn classify_external_ai_eligibility(
    sources: &RequiredClassificationSources,
) -> ClassificationEligibility {
    let resolved = resolve_inheritance(sources);
    eligibility_for_resolved(&resolved)
}

fn eligibility_for_resolved(resolved: &ResolvedClassification) -> ClassificationEligibility {
    match resolved.classification {
        DataClassification::Public => ClassificationEligibility::RequiresExternalAiGates {
            classification: DataClassification::Public,
        },
        DataClassification::Internal => {
            ClassificationEligibility::RequiresExplicitOrganizationPolicy {
                classification: DataClassification::Internal,
            }
        }
        DataClassification::Confidential => ClassificationEligibility::Denied {
            classification: DataClassification::Confidential,
            reason: ClassificationDenialReason::Confidential,
        },
        DataClassification::Restricted => ClassificationEligibility::Denied {
            classification: DataClassification::Restricted,
            reason: ClassificationDenialReason::Restricted,
        },
        DataClassification::Unclassified => ClassificationEligibility::Denied {
            classification: DataClassification::Unclassified,
            reason: ClassificationDenialReason::Unclassified,
        },
    }
}

trait ClassificationRank {
    fn rank(self) -> u8;
}

impl ClassificationRank for DataClassification {
    fn rank(self) -> u8 {
        match self {
            Self::Public => 0,
            Self::Internal => 1,
            Self::Confidential => 2,
            Self::Restricted => 3,
            Self::Unclassified => 4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete(values: &[DataClassification]) -> RequiredClassificationSources {
        let manifest: Vec<_> = (1..=values.len())
            .map(|id| RequiredSourceId(id as u64))
            .collect();
        let entries: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(index, classification)| {
                ClassificationSourceState::Trusted(TrustedClassificationSource::new(
                    RequiredSourceId(index as u64 + 1),
                    *classification,
                ))
            })
            .collect();
        valid(manifest, entries)
    }

    fn valid(
        manifest: Vec<RequiredSourceId>,
        entries: Vec<ClassificationSourceState>,
    ) -> RequiredClassificationSources {
        match RequiredClassificationSources::from_authority(manifest, entries) {
            Ok(snapshot) => snapshot,
            Err(_) => panic!("test authority fixture must validate"),
        }
    }

    #[test]
    fn complete_trusted_sources_use_most_restrictive_value() {
        let result = resolve_inheritance(&complete(&[
            DataClassification::Public,
            DataClassification::Internal,
            DataClassification::Confidential,
        ]));
        assert_eq!(result.classification(), DataClassification::Confidential);
        assert!(!result.fail_closed());
        assert_eq!(result.trusted_source_count(), 3);
    }

    #[test]
    fn missing_unknown_or_mismatched_required_identity_fails_closed() {
        let states = vec![
            ClassificationSourceState::Missing(RequiredSourceId(1)),
            ClassificationSourceState::Unknown(RequiredSourceId(2)),
            ClassificationSourceState::Trusted(TrustedClassificationSource::new(
                RequiredSourceId(3),
                DataClassification::Public,
            )),
        ];
        let result = resolve_inheritance(&valid(
            vec![
                RequiredSourceId(1),
                RequiredSourceId(2),
                RequiredSourceId(3),
            ],
            states,
        ));
        assert_eq!(result.classification(), DataClassification::Unclassified);
        assert!(result.fail_closed());
    }

    #[test]
    fn ai_suggestion_never_authorizes_or_overrides_trusted_value() {
        let sources = valid(
            vec![
                RequiredSourceId(1),
                RequiredSourceId(2),
                RequiredSourceId(3),
            ],
            vec![
                ClassificationSourceState::Trusted(TrustedClassificationSource::new(
                    RequiredSourceId(1),
                    DataClassification::Public,
                )),
                ClassificationSourceState::Trusted(TrustedClassificationSource::new(
                    RequiredSourceId(2),
                    DataClassification::Public,
                )),
                ClassificationSourceState::AiSuggestion(
                    RequiredSourceId(3),
                    DataClassification::Restricted,
                ),
            ],
        );
        let result = resolve_inheritance(&sources);
        assert_eq!(result.classification(), DataClassification::Unclassified);
        assert!(result.fail_closed());
    }

    #[test]
    fn eligibility_is_classification_only_and_has_no_allowed_or_payload_state() {
        let matrix = [
            (DataClassification::Public, true),
            (DataClassification::Internal, true),
            (DataClassification::Confidential, false),
            (DataClassification::Restricted, false),
            (DataClassification::Unclassified, false),
        ];
        for (classification, _) in matrix {
            let result = classify_external_ai_eligibility(&complete(&[
                classification,
                classification,
                classification,
            ]));
            match classification {
                DataClassification::Public => assert!(matches!(
                    result,
                    ClassificationEligibility::RequiresExternalAiGates {
                        classification: DataClassification::Public
                    }
                )),
                DataClassification::Internal => assert!(matches!(
                    result,
                    ClassificationEligibility::RequiresExplicitOrganizationPolicy {
                        classification: DataClassification::Internal
                    }
                )),
                DataClassification::Confidential => assert!(matches!(
                    result,
                    ClassificationEligibility::Denied {
                        classification: DataClassification::Confidential,
                        reason: ClassificationDenialReason::Confidential
                    }
                )),
                DataClassification::Restricted => assert!(matches!(
                    result,
                    ClassificationEligibility::Denied {
                        classification: DataClassification::Restricted,
                        reason: ClassificationDenialReason::Restricted
                    }
                )),
                DataClassification::Unclassified => assert!(matches!(
                    result,
                    ClassificationEligibility::Denied {
                        classification: DataClassification::Unclassified,
                        reason: ClassificationDenialReason::Unclassified
                    }
                )),
            }
        }
        let missing =
            classify_external_ai_eligibility(&RequiredClassificationSources::unavailable());
        assert!(matches!(
            missing,
            ClassificationEligibility::Denied {
                reason: ClassificationDenialReason::Unclassified,
                ..
            }
        ));
    }

    #[test]
    fn arbitrary_manifest_requires_every_source_including_a_fourth_restrictive_source() {
        let manifest = vec![
            RequiredSourceId(1),
            RequiredSourceId(2),
            RequiredSourceId(3),
            RequiredSourceId(4),
        ];
        let omitted = vec![
            ClassificationSourceState::Trusted(TrustedClassificationSource::new(
                RequiredSourceId(1),
                DataClassification::Public,
            )),
            ClassificationSourceState::Trusted(TrustedClassificationSource::new(
                RequiredSourceId(2),
                DataClassification::Public,
            )),
            ClassificationSourceState::Trusted(TrustedClassificationSource::new(
                RequiredSourceId(3),
                DataClassification::Public,
            )),
        ];
        assert_eq!(
            RequiredClassificationSources::from_authority(manifest.clone(), omitted).err(),
            Some(SnapshotValidationError::CountMismatch)
        );

        let complete_entries = vec![
            ClassificationSourceState::Trusted(TrustedClassificationSource::new(
                RequiredSourceId(1),
                DataClassification::Public,
            )),
            ClassificationSourceState::Trusted(TrustedClassificationSource::new(
                RequiredSourceId(2),
                DataClassification::Public,
            )),
            ClassificationSourceState::Trusted(TrustedClassificationSource::new(
                RequiredSourceId(3),
                DataClassification::Public,
            )),
            ClassificationSourceState::Trusted(TrustedClassificationSource::new(
                RequiredSourceId(4),
                DataClassification::Restricted,
            )),
        ];
        let resolved = resolve_inheritance(&valid(manifest, complete_entries));
        assert_eq!(resolved.classification(), DataClassification::Restricted);
        assert!(!resolved.fail_closed());
    }

    #[test]
    fn ordinary_write_table_is_monotonic_and_lowering_is_h2a_bound() {
        let context = OrdinaryWriteContext::ordinary();
        let internal = resolve_inheritance(&complete(&[
            DataClassification::Internal,
            DataClassification::Internal,
            DataClassification::Internal,
        ]));
        let confidential = resolve_inheritance(&complete(&[
            DataClassification::Confidential,
            DataClassification::Confidential,
            DataClassification::Confidential,
        ]));
        let restricted = resolve_inheritance(&complete(&[
            DataClassification::Restricted,
            DataClassification::Restricted,
            DataClassification::Restricted,
        ]));

        assert!(matches!(
            context.decide(DataClassification::Internal, &internal),
            OrdinaryWriteDecision::Applied {
                classification: DataClassification::Internal
            }
        ));
        assert!(matches!(
            context.decide(DataClassification::Public, &internal),
            OrdinaryWriteDecision::Applied {
                classification: DataClassification::Internal
            }
        ));
        assert!(matches!(
            context.decide(DataClassification::Internal, &confidential),
            OrdinaryWriteDecision::Applied {
                classification: DataClassification::Confidential
            }
        ));
        assert!(matches!(
            context.decide(DataClassification::Internal, &restricted),
            OrdinaryWriteDecision::Applied {
                classification: DataClassification::Restricted
            }
        ));

        let unclassified = resolve_inheritance(&RequiredClassificationSources::unavailable());
        assert!(matches!(
            context.decide(DataClassification::Unclassified, &internal),
            OrdinaryWriteDecision::Applied {
                classification: DataClassification::Internal
            }
        ));

        let OrdinaryWriteDecision::RequiresH2aLowering(requirement) =
            context.decide(DataClassification::Restricted, &internal)
        else {
            panic!("lowering must require H2a");
        };
        assert_eq!(requirement.current(), DataClassification::Restricted);
        assert_eq!(requirement.proposed(), DataClassification::Internal);
        assert!(requirement.requires_impact_preview());
        assert!(requirement.requires_rationale());
        assert!(requirement.requires_explicit_human_approval());
        assert!(requirement.requires_audit_event());

        assert!(matches!(
            OrdinaryWriteContext::from_authority(true)
                .decide(DataClassification::Internal, &unclassified),
            OrdinaryWriteDecision::Quarantined { .. }
        ));
    }
}
