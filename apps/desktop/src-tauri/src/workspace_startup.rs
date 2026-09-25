//! Which workspace this run opens (item ⑨; the accepted sample-workspace
//! amendment §2), decided before anything else starts: after the settings
//! are read and the sample's own interrupted reset or delete is finished,
//! and before restore reconciliation, backups, the Vault or the Ledger.

// The safe error envelope is every command's Err type by contract (DG3).
#![allow(clippy::result_large_err)]

use pmc_application::sample_workspace::SampleState;
use pmc_platform::settings::{SelectedWorkspace, StartupSelection};
use pmc_platform::workspace::WorkspaceKind;
use serde::Serialize;
use tauri::State;

use crate::safe_error::SafeErrorDto;

/// What this run opens.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Startup {
    Open(WorkspaceKind),
    /// A genuinely new profile: the first-run choice, and no Ledger, Vault,
    /// restore or backup work until it is made.
    FirstRun,
}

/// Whether the sample can be opened, from its reconciliation and what its
/// folder holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SampleAvailability {
    /// There, and PMC's own (current, or older and to be reset by the
    /// upgrade gate's `training_reset`).
    Available,
    /// An interrupted reset or delete could not be finished at startup.
    Unresolved,
    Missing,
    /// The sample folder holds something PMC did not write.
    Foreign,
}

impl SampleAvailability {
    #[must_use]
    pub fn from_state(reconciled: bool, state: Option<&SampleState>) -> Self {
        if !reconciled {
            return Self::Unresolved;
        }
        match state {
            None => Self::Unresolved,
            Some(SampleState::Current(_) | SampleState::Outdated(_)) => Self::Available,
            Some(SampleState::Absent) => Self::Missing,
            Some(SampleState::Foreign(_)) => Self::Foreign,
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unresolved => "unresolved",
            Self::Missing => "missing",
            Self::Foreign => "foreign",
        }
    }
}

/// Decide what this run opens. `developer_training` is the debug-only
/// `--pmc-workspace=training` seam; a release build always passes `false`.
/// It only stands in for a choice not made yet: a recorded choice — and the
/// Live that unreadable settings open — always wins. Chosen
/// sample data that cannot be opened opens Live for this run, and the
/// reason is kept for System Health — the choice itself is not changed.
#[must_use]
pub const fn decide(
    selection: StartupSelection,
    developer_training: bool,
    sample: SampleAvailability,
) -> Startup {
    match selection {
        StartupSelection::Choose if developer_training => Startup::Open(WorkspaceKind::Training),
        StartupSelection::Choose => Startup::FirstRun,
        StartupSelection::Open(SelectedWorkspace::Live) => Startup::Open(WorkspaceKind::Live),
        StartupSelection::Open(SelectedWorkspace::Training) => match sample {
            SampleAvailability::Available => Startup::Open(WorkspaceKind::Training),
            _ => Startup::Open(WorkspaceKind::Live),
        },
    }
}

/// What startup decided and found, for the shell, Settings and System
/// Health. Fixed for the run: a switch takes effect by restarting.
pub struct WorkspaceStatus {
    pub startup: Startup,
    pub selection: StartupSelection,
    pub sample: SampleAvailability,
    /// The settings document could not be read and was set aside.
    pub settings_set_aside: bool,
    /// The settings document could not be opened at all.
    pub settings_unavailable: bool,
    /// Folders a finished sample operation could not remove yet.
    pub sample_cleanup_pending: usize,
}

impl WorkspaceStatus {
    /// The workspace open this run; `None` during the first-run choice.
    #[must_use]
    pub const fn open(&self) -> Option<WorkspaceKind> {
        match self.startup {
            Startup::Open(kind) => Some(kind),
            Startup::FirstRun => None,
        }
    }

    /// Sample data was chosen but Live opened instead.
    #[must_use]
    pub fn sample_fell_back(&self) -> bool {
        self.selection == StartupSelection::Open(SelectedWorkspace::Training)
            && self.open() == Some(WorkspaceKind::Live)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStatusDto {
    /// `live` | `training` | `null` while the first-run choice is shown.
    pub open: Option<&'static str>,
    /// `live` | `training` | `null` when nothing is chosen yet.
    pub selected: Option<&'static str>,
    pub first_run: bool,
    /// `available` | `unresolved` | `missing` | `foreign`.
    pub sample: &'static str,
    /// Sample data was chosen but could not be opened; Live opened.
    pub sample_fell_back: bool,
    pub settings_set_aside: bool,
    pub settings_unavailable: bool,
    pub sample_cleanup_pending: usize,
}

const fn kind_name(kind: WorkspaceKind) -> &'static str {
    match kind {
        WorkspaceKind::Live => "live",
        WorkspaceKind::Training => "training",
    }
}

#[must_use]
pub fn status_dto(status: &WorkspaceStatus) -> WorkspaceStatusDto {
    WorkspaceStatusDto {
        open: status.open().map(kind_name),
        selected: match status.selection {
            StartupSelection::Open(SelectedWorkspace::Live) => Some("live"),
            StartupSelection::Open(SelectedWorkspace::Training) => Some("training"),
            StartupSelection::Choose => None,
        },
        first_run: status.startup == Startup::FirstRun,
        sample: status.sample.as_str(),
        sample_fell_back: status.sample_fell_back(),
        settings_set_aside: status.settings_set_aside,
        settings_unavailable: status.settings_unavailable,
        sample_cleanup_pending: status.sample_cleanup_pending,
    }
}

/// H0: which workspace is open and what startup found (§2, §5, System
/// Health). Read-only; no path.
#[tauri::command]
pub fn get_workspace_status(
    status: State<'_, WorkspaceStatus>,
) -> Result<WorkspaceStatusDto, SafeErrorDto> {
    Ok(status_dto(&status))
}

/// The window title's sample marker (§5), in the stored language; a
/// profile that follows the system gets English until the webview names
/// its own language.
#[must_use]
pub fn sample_title_suffix(locale: &str) -> &'static str {
    match locale {
        "zh-TW" => "範例資料",
        "zh-CN" => "示例数据",
        "ja" => "サンプルデータ",
        "ko" => "샘플 데이터",
        "es" => "Datos de ejemplo",
        _ => "Sample data",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: StartupSelection = StartupSelection::Open(SelectedWorkspace::Live);
    const SAMPLE: StartupSelection = StartupSelection::Open(SelectedWorkspace::Training);

    #[test]
    fn a_new_profile_chooses_and_a_chosen_workspace_opens() {
        use SampleAvailability::Available;
        assert_eq!(
            decide(StartupSelection::Choose, false, Available),
            Startup::FirstRun
        );
        assert_eq!(
            decide(LIVE, false, Available),
            Startup::Open(WorkspaceKind::Live)
        );
        assert_eq!(
            decide(SAMPLE, false, Available),
            Startup::Open(WorkspaceKind::Training)
        );
    }

    #[test]
    fn chosen_sample_data_that_cannot_open_opens_live_and_says_why() {
        for sample in [
            SampleAvailability::Unresolved,
            SampleAvailability::Missing,
            SampleAvailability::Foreign,
        ] {
            assert_eq!(
                decide(SAMPLE, false, sample),
                Startup::Open(WorkspaceKind::Live)
            );
            let status = WorkspaceStatus {
                startup: decide(SAMPLE, false, sample),
                selection: SAMPLE,
                sample,
                settings_set_aside: false,
                settings_unavailable: false,
                sample_cleanup_pending: 0,
            };
            assert!(status.sample_fell_back());
            assert!(status_dto(&status).sample_fell_back);
        }
    }

    #[test]
    fn the_developer_seam_only_stands_in_for_a_choice_not_made() {
        assert_eq!(
            decide(StartupSelection::Choose, true, SampleAvailability::Missing),
            Startup::Open(WorkspaceKind::Training)
        );
        // A recorded Live — and the Live unreadable settings open — wins.
        assert_eq!(
            decide(LIVE, true, SampleAvailability::Available),
            Startup::Open(WorkspaceKind::Live)
        );
    }

    #[test]
    fn a_reconcile_failure_is_unresolved() {
        assert_eq!(
            SampleAvailability::from_state(false, Some(&SampleState::Absent)),
            SampleAvailability::Unresolved
        );
        assert_eq!(
            SampleAvailability::from_state(true, Some(&SampleState::Absent)),
            SampleAvailability::Missing
        );
        assert_eq!(
            SampleAvailability::from_state(true, Some(&SampleState::Foreign(String::new()))),
            SampleAvailability::Foreign
        );
    }
}
