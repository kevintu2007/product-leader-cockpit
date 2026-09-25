//! Opens the real, on-disk Product Ledger and holds it as long-lived Tauri
//! state. This is the first production wiring between the desktop shell and
//! the backend crates, and it stands on its own: it does not depend on the
//! shared presentation-contract shell.
//!
//! `Live` is the default; `Training` is reachable only through the
//! debug-build launch switch in `lib::workspace_from_args`, and holds what
//! the `pmc-seed` tool writes.

use std::fs;
use std::sync::{Mutex, MutexGuard, PoisonError};

use pmc_application::evidence_writes::DesktopVault;
use pmc_domain::identity::CorrelationId;
use pmc_domain::time::UtcTimestamp;

use crate::backup_gate::{BackupGate, GateAdmission, GateExclusive, GateRefusal};
use crate::display_settings::SettingsState;
use crate::safe_error::SafeErrorDto;
use pmc_ledger::sqlite::{LedgerOpenError, SqliteProductLedger};
use pmc_platform::settings::{DeferredDirectoryPath, ProtectedSettingsRoot};
use pmc_platform::workspace::{WorkspaceIdentity, WorkspaceKind};

/// Deliberately distinct from both `tauri.conf.json`'s bare `productName`
/// and its `identifier` (`com.productmissioncontrol.desktop`): the webview
/// runtime already owns an app-data directory under the identifier for its
/// own cookies/local-storage state, and a plain "ProductMissionControl"
/// collided with an unrelated pre-existing directory found on a real dev
/// machine during startup verification. This name is intentionally unique
/// to our own protected settings root.
const APPLICATION_DIRECTORY: &str = "ProductMissionControlDesktop";
const LEDGER_FILE_NAME: &str = "product-ledger.sqlite3";

/// Why the Ledger is not open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnavailableReason {
    /// A restore may have left the Ledger half replaced; only System
    /// Health's recovery is offered (DG3 restore amendment §2).
    RestoreRecoveryRequired,
    /// The restored Ledger is from an older schema and waits for the upgrade
    /// gate (8f); product owner 2026-09-22: no silent upgrade in a restore.
    UpgradeRequired,
    /// Made by a development build older than the first supported format.
    UnsupportedOld,
    /// Made by a newer PMC (DG3 upgrade-gate amendment §2): no downgrade.
    NewerVersion,
    /// It did not open for another reason.
    OpenFailed,
    /// A new profile has not chosen its workspace yet: nothing is opened
    /// until it does (sample-workspace amendment §2).
    FirstRun,
}

impl UnavailableReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RestoreRecoveryRequired => "restore_recovery_required",
            Self::UpgradeRequired => "upgrade_required",
            Self::UnsupportedOld => "unsupported_old",
            Self::NewerVersion => "newer_version",
            Self::OpenFailed => "open_failed",
            Self::FirstRun => "first_run",
        }
    }
}

/// Where the Ledger is in its life (the 8e design; 8f adds the upgrade
/// gate's states). Only `Ready` holds an open connection.
pub enum LedgerLifecycle {
    Ready(SqliteProductLedger),
    /// A restore closed it and is replacing the file.
    Replacing,
    Unavailable(UnavailableReason),
}

/// The open Ledger. Its mutex is private: a command reads through
/// [`LedgerState::read`], which cannot mutate, or writes through
/// [`LedgerState::write`], which first passes the backup gate (S7 §6). No
/// other way in exists, so a new write command cannot forget the gate —
/// and neither can reach a Ledger that is closed for a restore.
pub struct LedgerState(Mutex<LedgerLifecycle>);

/// The Ledger is not open now: being replaced, or unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerClosed {
    Replacing,
    Unavailable(UnavailableReason),
}

impl LedgerClosed {
    pub fn to_safe_error(self, correlation: &CorrelationId) -> SafeErrorDto {
        match self {
            Self::Replacing => GateRefusal::Restoring.to_safe_error(correlation),
            Self::Unavailable(_) => GateRefusal::LedgerUnavailable.to_safe_error(correlation),
        }
    }

    /// `replacing` or the unavailable reason, for the webview.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Replacing => "replacing",
            Self::Unavailable(reason) => reason.as_str(),
        }
    }
}

/// Shared, read-only access for one command. Derefs to `&SqliteProductLedger`
/// only; built only from a `Ready` lifecycle.
pub struct LedgerReadGuard<'a>(MutexGuard<'a, LedgerLifecycle>);

fn ready(lifecycle: &LedgerLifecycle) -> &SqliteProductLedger {
    match lifecycle {
        LedgerLifecycle::Ready(ledger) => ledger,
        // Guards are only built after checking `Ready`, under the lock.
        LedgerLifecycle::Replacing | LedgerLifecycle::Unavailable(_) => unreachable!(),
    }
}

impl std::ops::Deref for LedgerReadGuard<'_> {
    type Target = SqliteProductLedger;
    fn deref(&self) -> &SqliteProductLedger {
        ready(&self.0)
    }
}

/// Write access for one command: holds the gate's admission and the Ledger
/// together for the whole transaction, so a backup cannot start between the
/// check and the commit (lock order: gate, then Ledger — backups take the
/// same order).
pub struct LedgerWriteGuard<'a> {
    ledger: MutexGuard<'a, LedgerLifecycle>,
    _admission: GateAdmission<'a>,
}

impl std::ops::Deref for LedgerWriteGuard<'_> {
    type Target = SqliteProductLedger;
    fn deref(&self) -> &SqliteProductLedger {
        ready(&self.ledger)
    }
}

impl std::ops::DerefMut for LedgerWriteGuard<'_> {
    fn deref_mut(&mut self) -> &mut SqliteProductLedger {
        match &mut *self.ledger {
            LedgerLifecycle::Ready(ledger) => ledger,
            LedgerLifecycle::Replacing | LedgerLifecycle::Unavailable(_) => unreachable!(),
        }
    }
}

impl LedgerState {
    pub fn new(ledger: SqliteProductLedger) -> Self {
        Self(Mutex::new(LedgerLifecycle::Ready(ledger)))
    }

    /// No Ledger was opened (startup found it must not be).
    pub fn unavailable(reason: UnavailableReason) -> Self {
        Self(Mutex::new(LedgerLifecycle::Unavailable(reason)))
    }

    fn lock(&self) -> MutexGuard<'_, LedgerLifecycle> {
        // A poisoned mutex is recovered rather than propagated: the SQLite
        // transaction that was in flight has already rolled back.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn open_guard(&self) -> Result<MutexGuard<'_, LedgerLifecycle>, LedgerClosed> {
        let guard = self.lock();
        match &*guard {
            LedgerLifecycle::Ready(_) => Ok(guard),
            LedgerLifecycle::Replacing => Err(LedgerClosed::Replacing),
            LedgerLifecycle::Unavailable(reason) => Err(LedgerClosed::Unavailable(*reason)),
        }
    }

    /// The Ledger for reading, held only for one command.
    pub fn read(&self) -> Result<LedgerReadGuard<'_>, LedgerClosed> {
        self.open_guard().map(LedgerReadGuard)
    }

    /// The Ledger for one write, if the backup gate admits it now.
    pub fn write<'a>(
        &'a self,
        gate: &'a BackupGate,
        now: UtcTimestamp,
    ) -> Result<LedgerWriteGuard<'a>, GateRefusal> {
        let admission = gate.admit(now)?;
        let ledger = self.open_guard().map_err(|closed| match closed {
            LedgerClosed::Replacing => GateRefusal::Restoring,
            LedgerClosed::Unavailable(_) => GateRefusal::LedgerUnavailable,
        })?;
        Ok(LedgerWriteGuard {
            ledger,
            _admission: admission,
        })
    }

    /// The Ledger for a backup snapshot, taken while the backup holds the
    /// gate exclusively. Read-only, like [`LedgerState::read`]; it exists so
    /// the lock order is visible where it matters.
    pub fn read_for_snapshot<'a>(
        &'a self,
        _exclusive: &GateExclusive<'a>,
    ) -> Result<LedgerReadGuard<'a>, LedgerClosed> {
        self.read()
    }

    /// Close the Ledger for a restore, under the gate's exclusive hold so no
    /// write is in flight. Dropping the connection checkpoints and closes the
    /// last handle this process holds.
    pub fn close_for_restore(&self, _exclusive: &GateExclusive<'_>) -> Result<(), LedgerClosed> {
        let mut guard = self.open_guard()?;
        *guard = LedgerLifecycle::Replacing;
        Ok(())
    }

    /// A Ledger that was never opened because it is `from` (a restore from
    /// the upgrade gate) is being replaced now. Only from exactly that state,
    /// under the gate's exclusive hold; `false` changes nothing.
    pub fn begin_replacing(&self, from: UnavailableReason, _exclusive: &GateExclusive<'_>) -> bool {
        let mut guard = self.lock();
        if matches!(&*guard, LedgerLifecycle::Unavailable(reason) if *reason == from) {
            *guard = LedgerLifecycle::Replacing;
            true
        } else {
            false
        }
    }

    /// Put an open Ledger back (after a restore, or a recovery).
    pub fn install(&self, ledger: SqliteProductLedger) {
        *self.lock() = LedgerLifecycle::Ready(ledger);
    }

    /// Mark the Ledger unavailable (a restore that could not be put back).
    pub fn set_unavailable(&self, reason: UnavailableReason) {
        *self.lock() = LedgerLifecycle::Unavailable(reason);
    }

    /// The lifecycle's state, for the webview.
    pub fn state(&self) -> Result<(), LedgerClosed> {
        self.open_guard().map(|_| ())
    }
}

/// Where this workspace's Product Vault comes from, held beside the Ledger.
/// Unlike the Ledger this needs no mutex: the candidate is derived (Training)
/// or read from settings (Live) for each command, and the validated handle it
/// produces is created and dropped inside that one command.
///
/// Live is read from settings per operation on purpose (item ⑦): the folder
/// a person chooses takes effect without a restart, and a long-lived copy
/// could disagree with the document another part of the host just wrote.
pub struct VaultState {
    kind: WorkspaceKind,
    /// Training's derived Vault; `None` for Live.
    synthetic: DesktopVault,
    /// An interrupted change of the Live Vault folder could not be resolved
    /// at startup: no Vault is used this run (item ⑦, fail closed).
    change_unresolved: bool,
}

impl VaultState {
    pub const fn new(
        kind: WorkspaceKind,
        synthetic: DesktopVault,
        change_unresolved: bool,
    ) -> Self {
        Self {
            kind,
            synthetic,
            change_unresolved,
        }
    }

    /// The Vault to use for one operation now.
    pub fn current(&self, settings: &SettingsState) -> DesktopVault {
        self.snapshot(settings).0
    }

    /// The Vault and the folder's own name, from **one** read of the
    /// settings. A surface that showed availability from one read and the
    /// name from another could name one folder while describing another,
    /// because a Vault change can commit between them.
    ///
    /// The name is `None` for Training, whose Vault the person did not
    /// choose, and when none is set.
    pub fn snapshot(&self, settings: &SettingsState) -> (DesktopVault, Option<String>) {
        match self.kind {
            WorkspaceKind::Training => (self.synthetic.clone(), None),
            WorkspaceKind::Live if self.change_unresolved => {
                (DesktopVault::change_unresolved(), None)
            }
            WorkspaceKind::Live => {
                let folder = self.live_root(settings);
                let name = folder.as_ref().and_then(DeferredDirectoryPath::folder_name);
                (DesktopVault::configured(folder.as_ref()), name)
            }
        }
    }

    /// The Live Vault folder as the settings hold it right now, or `None`
    /// when none is configured or the settings cannot be read (which is not
    /// a configured Vault either, and says so).
    fn live_root(&self, settings: &SettingsState) -> Option<DeferredDirectoryPath> {
        let guard = settings
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.as_ref()?.read().ok()?.operational.live_vault_root
    }
}

/// The protected, app-owned root every workspace and the settings document
/// live under. Fatal when missing, like the Ledger itself.
pub fn protected_root() -> ProtectedSettingsRoot {
    ProtectedSettingsRoot::prepare(APPLICATION_DIRECTORY)
        .unwrap_or_else(|error| panic!("PLATFORM_SETTINGS_ROOT_FAILED: {error}"))
}

/// Whether this profile has used the app before: its Live workspace
/// directory exists. Only Live counts; a developer's Training workspace says
/// nothing about the person's own profile. Checked before anything creates
/// the directory. A profile whose Live workspace was deleted reads as new.
pub fn live_profile_exists(protected_root: &ProtectedSettingsRoot) -> bool {
    WorkspaceIdentity::resolve(protected_root, WorkspaceKind::Live)
        .is_ok_and(|workspace| workspace.root().as_path().exists())
}

/// Resolve the protected, app-owned workspace directory of the given kind,
/// open (or create) the real Product Ledger inside it, and derive the
/// candidate Product Vault beside it.
///
/// The two stores fail differently on purpose. A Ledger that does not open
/// is returned as the error, for the caller to classify: an older format
/// goes to the upgrade gate, anything else to System Health (DG3 upgrade-gate
/// amendment §2). A missing Vault is
/// **ordinary**: every read route and every Ledger-only write still works,
/// and the Vault may appear later when the person runs `pmc-seed`. So
/// [`DesktopVault`] is derived without touching the filesystem and validates
/// itself per operation; nothing here can fail because of it.
pub fn open_workspace(
    protected_root: &ProtectedSettingsRoot,
    kind: WorkspaceKind,
) -> (Result<SqliteProductLedger, LedgerOpenError>, DesktopVault) {
    let workspace = WorkspaceIdentity::resolve(protected_root, kind)
        .unwrap_or_else(|error| panic!("PLATFORM_WORKSPACE_RESOLVE_FAILED: {error}"));
    // `WorkspaceIdentity::resolve` deliberately never creates the directory
    // itself (see its own doc comment) -- only validates that any existing
    // path segment is safe. Creating it here, after that validation, keeps
    // the "no raw caller path" containment guarantee intact.
    fs::create_dir_all(workspace.root().as_path())
        .unwrap_or_else(|error| panic!("PLATFORM_WORKSPACE_CREATE_FAILED: {error}"));
    let ledger_path = workspace.root().as_path().join(LEDGER_FILE_NAME);
    // A Ledger that does not open is classified by the caller (the upgrade
    // gate or System Health), never a crash.
    let ledger = SqliteProductLedger::open(&ledger_path);
    let vault = DesktopVault::resolve(&workspace);
    (ledger, vault)
}

/// The candidate Product Vault alone, for a start that must not open the
/// Ledger. Touches no file, like [`open_workspace`]'s Vault.
pub fn resolve_vault(protected_root: &ProtectedSettingsRoot, kind: WorkspaceKind) -> DesktopVault {
    let workspace = WorkspaceIdentity::resolve(protected_root, kind)
        .unwrap_or_else(|error| panic!("PLATFORM_WORKSPACE_RESOLVE_FAILED: {error}"));
    DesktopVault::resolve(&workspace)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn ledger() -> SqliteProductLedger {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        SqliteProductLedger::open(
            std::env::temp_dir().join(format!("pmc-ledger-state-{nonce}.sqlite3")),
        )
        .unwrap()
    }

    #[test]
    fn a_ledger_closed_for_a_restore_refuses_reads_and_writes_until_reinstalled() {
        let state = LedgerState::new(ledger());
        let gate = BackupGate::new(false);
        {
            let (_ticket, exclusive) = gate.begin_restore().unwrap();
            state.close_for_restore(&exclusive).unwrap();
        }
        assert_eq!(state.read().err(), Some(LedgerClosed::Replacing));
        assert_eq!(
            state.write(&gate, UtcTimestamp::from_unix_millis(0)).err(),
            Some(GateRefusal::Restoring)
        );
        state.install(ledger());
        assert!(state.read().is_ok());
        assert!(state
            .write(&gate, UtcTimestamp::from_unix_millis(0))
            .is_ok());
    }

    #[test]
    fn an_unavailable_ledger_says_why() {
        let state = LedgerState::unavailable(UnavailableReason::RestoreRecoveryRequired);
        let closed = state.read().err().unwrap();
        assert_eq!(closed.as_str(), "restore_recovery_required");
        assert_eq!(
            state
                .write(&BackupGate::new(false), UtcTimestamp::from_unix_millis(0))
                .err(),
            Some(GateRefusal::LedgerUnavailable)
        );
    }
}
