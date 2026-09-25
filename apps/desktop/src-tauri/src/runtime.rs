//! Host-minted identifiers for failures the desktop itself originates.
//!
//! A read command has no caller-supplied correlation id, yet DG3's Error
//! Contract requires one on every safe error so the O05 copy button has
//! something traceable. The host therefore mints `host-<launch nonce>-<n>`:
//! the nonce distinguishes launches, the counter distinguishes calls, and
//! neither encodes anything about the user or the data.
//!
//! This is not the write path's id source. Idempotency, entity, prepared
//! intent, audit and receipt ids for writes are a later slice's concern and
//! are minted where the domain says they are.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use pmc_application::desktop_runtime::{OpaqueIdSource, SystemClock};
use pmc_domain::identity::CorrelationId;
use pmc_domain::time::UtcTimestamp;

/// The host's write-path runtime: one opaque id source for the process and
/// the system clock. The webview never supplies an id the host mints, nor a
/// timestamp. Managed as Tauri state next to the Ledger.
pub struct HostRuntime {
    ids: Mutex<OpaqueIdSource>,
    clock: SystemClock,
}

impl HostRuntime {
    pub fn new() -> Self {
        Self {
            ids: Mutex::new(OpaqueIdSource::new()),
            clock: SystemClock,
        }
    }

    /// The id source, held only for the duration of one command.
    pub fn ids(&self) -> MutexGuard<'_, OpaqueIdSource> {
        self.ids.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn now(&self) -> UtcTimestamp {
        self.clock.read()
    }
}

impl Default for HostRuntime {
    fn default() -> Self {
        Self::new()
    }
}

static LAUNCH_NONCE: OnceLock<u64> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);

fn launch_nonce() -> u64 {
    *LAUNCH_NONCE.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos() as u64)
    })
}

/// A fresh host correlation id for one command invocation.
pub fn host_correlation() -> CorrelationId {
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let candidate = format!("host-{:x}-{sequence}", launch_nonce());
    // The candidate is a token of ASCII hex, digits and hyphens well under
    // the identifier limit; parsing it cannot fail. If that ever changes
    // the fallback is a constant id, which is still a valid, if less
    // distinguishing, correlation for a host-originated error.
    CorrelationId::parse(candidate).unwrap_or_else(|_| {
        CorrelationId::parse("host-correlation").unwrap_or_else(|_| unreachable!())
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn host_correlations_are_distinct_and_carry_no_content() {
        let first = host_correlation();
        let second = host_correlation();
        assert_ne!(first, second);
        let rest = first
            .as_str()
            .strip_prefix("host-")
            .expect("host correlations carry the host prefix");
        assert!(rest
            .chars()
            .all(|character| character.is_ascii_hexdigit() || character == '-'));
    }
}
