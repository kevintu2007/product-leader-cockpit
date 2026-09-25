//! The backup-due write gate (S7 plan §6; DG3 backup-setup amendment §5).
//!
//! In the Live workspace no Ledger write is admitted while an Operational
//! Backup is due (none verified in the last 24 hours), while the startup
//! check of existing backups is still running, or while a backup runs. The
//! Training workspace holds synthetic data only and is never gated.
//!
//! Admission is decided on cached state and the clock only: the registry's
//! archives are re-hashed at startup and after each backup, never per write,
//! so a slow or unplugged drive cannot stall ordinary work (design review
//! 2026-09-22). A write keeps its [`GateAdmission`] — the gate's mutex — for
//! its whole transaction; a backup takes the same mutex before it marks
//! itself running and before it touches the Ledger, so no write can slip
//! between a check and a commit.

use std::sync::{Mutex, MutexGuard, PoisonError};

use pmc_domain::identity::CorrelationId;
use pmc_domain::time::UtcTimestamp;
use pmc_platform::backup_registry::{backup_due, DUE_AFTER_MILLIS};

use crate::safe_error::SafeErrorDto;

/// How a finished run ended, kept until the next run replaces it.
#[derive(Clone, Debug)]
pub struct LastRun {
    pub run_id: u64,
    /// `None` when the backup verified and was registered.
    pub failure_key: Option<&'static str>,
}

pub struct GateInner {
    enforced: bool,
    reconciled: bool,
    valid_verified_at: Vec<i64>,
    running: bool,
    /// A restore holds the Ledger closed (DG3 restore amendment §4).
    restoring: bool,
    orphan_count: usize,
    last_run: Option<LastRun>,
    next_run_id: u64,
}

/// Two locks, always taken in this order: `admission` (held by a write for
/// its whole transaction, and by a backup while it takes its snapshot), then
/// `state` (held only for a moment, to read or change it). A run ticket
/// ending — even while unwinding, even while its exclusive hold is still
/// alive — takes only `state`, so no drop order can deadlock.
pub struct BackupGate {
    admission: Mutex<()>,
    state: Mutex<GateInner>,
}

/// Held by one admitted write for its whole transaction.
pub struct GateAdmission<'a>(#[allow(dead_code)] MutexGuard<'a, ()>);

/// Held by a backup while it marks itself running and takes its snapshot.
pub struct GateExclusive<'a>(#[allow(dead_code)] MutexGuard<'a, ()>);

/// One running backup. [`RunTicket::finish`] records how it ended; dropping
/// it unfinished records a failure.
pub struct RunTicket<'a> {
    gate: &'a BackupGate,
    run_id: u64,
    finished: bool,
}

impl RunTicket<'_> {
    pub fn finish(mut self, outcome: Result<i64, &'static str>) {
        self.finished = true;
        self.gate.finish_run(self.run_id, outcome);
    }
}

impl Drop for RunTicket<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.gate
                .finish_run(self.run_id, Err("desktop.backup_failed"));
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateRefusal {
    /// A backup is due (or the startup check has not finished).
    Due,
    /// A backup is running now.
    Running,
    /// A restore is running now.
    Restoring,
    /// The Ledger could not be opened; System Health says why.
    LedgerUnavailable,
}

impl GateRefusal {
    pub fn to_safe_error(self, correlation: &CorrelationId) -> SafeErrorDto {
        match self {
            Self::Due => SafeErrorDto::host("BACKUP_DUE", "desktop.backup_due", correlation, true),
            Self::Running => SafeErrorDto::host(
                "BACKUP_RUNNING",
                "desktop.backup_running",
                correlation,
                true,
            ),
            Self::Restoring => SafeErrorDto::host(
                "RESTORE_RUNNING",
                "desktop.restore_running",
                correlation,
                true,
            ),
            Self::LedgerUnavailable => SafeErrorDto::host(
                "LEDGER_UNAVAILABLE",
                "desktop.ledger_unavailable",
                correlation,
                false,
            ),
        }
    }
}

/// One running restore; dropping it, however the restore ended, reopens
/// admission to the gate's own rules.
pub struct RestoreTicket<'a> {
    gate: &'a BackupGate,
}

impl Drop for RestoreTicket<'_> {
    fn drop(&mut self) {
        self.gate.lock().restoring = false;
    }
}

/// The one state the webview is told (DG3 amendment §2, §5).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GateState {
    /// Training: never gated.
    NotRequired,
    /// Startup is still checking the recorded backups.
    Checking,
    BackingUp,
    Restoring,
    Due,
    Current,
}

impl GateState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotRequired => "not_required",
            Self::Checking => "checking",
            Self::BackingUp => "backing_up",
            Self::Restoring => "restoring",
            Self::Due => "due",
            Self::Current => "current",
        }
    }
}

/// A consistent view for `get_backup_status`.
#[derive(Clone, Debug)]
pub struct GateView {
    pub state: GateState,
    pub last_verified_at_millis: Option<i64>,
    pub next_due_at_millis: Option<i64>,
    pub orphan_count: usize,
    pub last_run: Option<LastRun>,
}

impl BackupGate {
    /// `enforced` is true for the Live workspace. Until [`reconciled`] runs,
    /// an enforced gate refuses every write.
    ///
    /// [`reconciled`]: BackupGate::reconciled
    pub fn new(enforced: bool) -> Self {
        Self {
            admission: Mutex::new(()),
            state: Mutex::new(GateInner {
                enforced,
                reconciled: false,
                valid_verified_at: Vec::new(),
                running: false,
                restoring: false,
                orphan_count: 0,
                last_run: None,
                next_run_id: 1,
            }),
        }
    }

    fn admission(&self) -> MutexGuard<'_, ()> {
        self.admission
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn lock(&self) -> MutexGuard<'_, GateInner> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn state_of(inner: &GateInner, now_millis: i64) -> GateState {
        if inner.restoring {
            GateState::Restoring
        } else if !inner.enforced {
            GateState::NotRequired
        } else if inner.running {
            GateState::BackingUp
        } else if !inner.reconciled {
            GateState::Checking
        } else if backup_due(inner.valid_verified_at.iter().copied(), now_millis) {
            GateState::Due
        } else {
            GateState::Current
        }
    }

    /// Admit one write now, or say why not.
    pub fn admit(&self, now: UtcTimestamp) -> Result<GateAdmission<'_>, GateRefusal> {
        let admission = self.admission();
        let state = Self::state_of(&self.lock(), now.unix_millis());
        match state {
            GateState::NotRequired | GateState::Current => Ok(GateAdmission(admission)),
            GateState::BackingUp => Err(GateRefusal::Running),
            GateState::Restoring => Err(GateRefusal::Restoring),
            GateState::Checking | GateState::Due => Err(GateRefusal::Due),
        }
    }

    /// Start a backup: refused while one runs, and until the startup check
    /// has finished (it empties the same work area). The exclusive hold must
    /// be dropped once the snapshot is taken; the run stays marked running
    /// until its [`RunTicket`] is finished — or dropped, which ends it as
    /// failed, so a cancelled or panicking run never leaves writes closed.
    ///
    /// Ending the ticket takes only the state lock, so it never waits on the
    /// exclusive hold, whatever the drop order.
    pub fn begin_run(&self) -> Result<(RunTicket<'_>, GateExclusive<'_>), GateRefusal> {
        // Waits for any admitted write to commit.
        let admission = self.admission();
        let run_id = {
            let mut inner = self.lock();
            if inner.restoring {
                return Err(GateRefusal::Restoring);
            }
            if inner.running || !inner.reconciled {
                return Err(GateRefusal::Running);
            }
            inner.running = true;
            let run_id = inner.next_run_id;
            inner.next_run_id += 1;
            run_id
        };
        Ok((
            RunTicket {
                gate: self,
                run_id,
                finished: false,
            },
            GateExclusive(admission),
        ))
    }

    /// Start replacing the Ledger: refused while a backup or another restore
    /// runs. Writes are refused as "restoring" until the ticket drops; the
    /// exclusive hold waits for any admitted write, and is where the Ledger
    /// is closed. Any workspace: the gate's enforcement does not apply.
    pub fn begin_restore(&self) -> Result<(RestoreTicket<'_>, GateExclusive<'_>), GateRefusal> {
        let admission = self.admission();
        let mut inner = self.lock();
        if inner.restoring {
            return Err(GateRefusal::Restoring);
        }
        if inner.running {
            return Err(GateRefusal::Running);
        }
        inner.restoring = true;
        drop(inner);
        Ok((RestoreTicket { gate: self }, GateExclusive(admission)))
    }

    /// End a run: with the new verification time on success, or the failure
    /// key.
    fn finish_run(&self, run_id: u64, outcome: Result<i64, &'static str>) {
        let mut inner = self.lock();
        inner.running = false;
        let failure_key = match outcome {
            Ok(verified_at) => {
                inner.valid_verified_at.push(verified_at);
                None
            }
            Err(key) => Some(key),
        };
        inner.last_run = Some(LastRun {
            run_id,
            failure_key,
        });
    }

    /// Record the startup check: the verification times of the records whose
    /// archives are still byte-for-byte what was verified, and how many
    /// unknown archives the destination holds.
    pub fn reconciled(&self, valid_verified_at: Vec<i64>, orphan_count: usize) {
        let mut inner = self.lock();
        inner.valid_verified_at = valid_verified_at;
        inner.orphan_count = orphan_count;
        inner.reconciled = true;
    }

    pub fn view(&self, now: UtcTimestamp) -> GateView {
        let inner = self.lock();
        let now_millis = now.unix_millis();
        // Only times not in the future count, as for `backup_due`.
        let last = inner
            .valid_verified_at
            .iter()
            .copied()
            .filter(|verified_at| *verified_at <= now_millis)
            .max();
        GateView {
            state: Self::state_of(&inner, now_millis),
            last_verified_at_millis: last,
            next_due_at_millis: last.map(|verified_at| verified_at + DUE_AFTER_MILLIS),
            orphan_count: inner.orphan_count,
            last_run: inner.last_run.clone(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const HOUR: i64 = 60 * 60 * 1000;

    fn at(millis: i64) -> UtcTimestamp {
        UtcTimestamp::from_unix_millis(millis)
    }

    #[test]
    fn a_live_gate_refuses_until_the_startup_check_and_a_fresh_backup() {
        let gate = BackupGate::new(true);
        assert_eq!(gate.admit(at(100 * HOUR)).err(), Some(GateRefusal::Due));
        gate.reconciled(Vec::new(), 0);
        assert_eq!(gate.admit(at(100 * HOUR)).err(), Some(GateRefusal::Due));
        gate.reconciled(vec![99 * HOUR], 0);
        assert!(gate.admit(at(100 * HOUR)).is_ok());
        // Crossing 24 hours mid-session closes it again.
        assert_eq!(gate.admit(at(124 * HOUR)).err(), Some(GateRefusal::Due));
    }

    #[test]
    fn training_is_never_gated() {
        let gate = BackupGate::new(false);
        assert!(gate.admit(at(0)).is_ok());
        assert_eq!(gate.view(at(0)).state, GateState::NotRequired);
    }

    #[test]
    fn a_running_backup_refuses_writes_and_a_second_run() {
        let gate = BackupGate::new(true);
        gate.reconciled(vec![99 * HOUR], 0);
        let (ticket, exclusive) = gate.begin_run().unwrap();
        drop(exclusive);
        assert_eq!(gate.admit(at(100 * HOUR)).err(), Some(GateRefusal::Running));
        assert!(gate.begin_run().is_err());
        assert_eq!(gate.view(at(100 * HOUR)).state, GateState::BackingUp);
        ticket.finish(Err("desktop.backup_failed"));
        assert!(gate.admit(at(100 * HOUR)).is_ok());
        let view = gate.view(at(100 * HOUR));
        assert_eq!(
            view.last_run.unwrap().failure_key,
            Some("desktop.backup_failed")
        );
    }

    #[test]
    fn a_verified_run_opens_a_due_gate_and_sets_the_next_due_time() {
        let gate = BackupGate::new(true);
        gate.reconciled(Vec::new(), 2);
        let (ticket, exclusive) = gate.begin_run().unwrap();
        drop(exclusive);
        ticket.finish(Ok(100 * HOUR));
        assert!(gate.admit(at(101 * HOUR)).is_ok());
        let view = gate.view(at(101 * HOUR));
        assert_eq!(view.state, GateState::Current);
        assert_eq!(view.last_verified_at_millis, Some(100 * HOUR));
        assert_eq!(view.next_due_at_millis, Some(124 * HOUR));
        assert_eq!(view.orphan_count, 2);
    }

    #[test]
    fn no_backup_starts_before_the_startup_check_has_finished() {
        let gate = BackupGate::new(true);
        assert!(gate.begin_run().is_err());
        gate.reconciled(Vec::new(), 0);
        assert!(gate.begin_run().is_ok());
    }

    #[test]
    fn a_run_dropped_unfinished_ends_as_failed_and_reopens_the_gate() {
        let gate = BackupGate::new(true);
        gate.reconciled(vec![99 * HOUR], 0);
        {
            let (_ticket, _exclusive) = gate.begin_run().unwrap();
            // Cancelled or panicked here.
        }
        assert!(gate.admit(at(100 * HOUR)).is_ok());
        assert_eq!(
            gate.view(at(100 * HOUR)).last_run.unwrap().failure_key,
            Some("desktop.backup_failed")
        );
    }

    #[test]
    fn a_verification_time_in_the_future_is_neither_fresh_nor_shown() {
        let gate = BackupGate::new(true);
        gate.reconciled(vec![200 * HOUR], 0);
        assert_eq!(gate.admit(at(100 * HOUR)).err(), Some(GateRefusal::Due));
        assert_eq!(gate.view(at(100 * HOUR)).last_verified_at_millis, None);
    }

    #[test]
    fn a_write_in_progress_holds_the_gate_so_a_backup_waits_for_it() {
        let gate = std::sync::Arc::new(BackupGate::new(true));
        gate.reconciled(vec![99 * HOUR], 0);
        let admission = gate.admit(at(100 * HOUR)).unwrap();
        let other = std::sync::Arc::clone(&gate);
        let started = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&started);
        let handle = std::thread::spawn(move || {
            let (_ticket, _exclusive) = other.begin_run().unwrap();
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!started.load(std::sync::atomic::Ordering::SeqCst));
        drop(admission);
        handle.join().unwrap();
        assert!(started.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn a_restore_refuses_writes_and_backups_in_any_workspace_until_it_ends() {
        for enforced in [true, false] {
            let gate = BackupGate::new(enforced);
            gate.reconciled(vec![99 * HOUR], 0);
            let (ticket, exclusive) = gate.begin_restore().unwrap();
            drop(exclusive);
            assert_eq!(
                gate.admit(at(100 * HOUR)).err(),
                Some(GateRefusal::Restoring)
            );
            assert_eq!(gate.begin_run().err(), Some(GateRefusal::Restoring));
            assert_eq!(gate.begin_restore().err(), Some(GateRefusal::Restoring));
            assert_eq!(gate.view(at(100 * HOUR)).state, GateState::Restoring);
            drop(ticket);
            assert!(gate.admit(at(100 * HOUR)).is_ok());
        }
    }

    #[test]
    fn no_restore_starts_while_a_backup_runs() {
        let gate = BackupGate::new(true);
        gate.reconciled(vec![99 * HOUR], 0);
        let (_ticket, exclusive) = gate.begin_run().unwrap();
        drop(exclusive);
        assert_eq!(gate.begin_restore().err(), Some(GateRefusal::Running));
    }
}
