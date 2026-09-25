#![cfg(target_os = "windows")]

use std::time::{SystemTime, UNIX_EPOCH};

use pmc_platform::credential::{CredentialId, WindowsCredentialStore};

struct CredentialCleanup<'a> {
    store: &'a WindowsCredentialStore,
    id: &'a CredentialId,
    armed: bool,
}

impl CredentialCleanup<'_> {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for CredentialCleanup<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _cleanup_result = self.store.delete(self.id);
        }
    }
}

#[test]
fn synthetic_secret_round_trips_only_through_windows_credential_manager() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let id = CredentialId::new(
        "product-mission-control.synthetic-test",
        format!("credential-{nonce}"),
    )
    .unwrap_or_else(|error| panic!("credential ID failed: {error}"));
    let store = WindowsCredentialStore::open()
        .unwrap_or_else(|error| panic!("credential store failed: {error}"));
    let synthetic_secret = b"public-safe-synthetic-credential";

    store
        .set(&id, synthetic_secret)
        .unwrap_or_else(|error| panic!("credential write failed: {error}"));
    let mut cleanup = CredentialCleanup {
        store: &store,
        id: &id,
        armed: true,
    };
    let loaded = store
        .get(&id)
        .unwrap_or_else(|error| panic!("credential read failed: {error}"));
    assert_eq!(loaded.expose(), synthetic_secret);
    drop(loaded);
    store
        .delete(&id)
        .unwrap_or_else(|error| panic!("credential cleanup failed: {error}"));
    cleanup.disarm();
    assert!(store.get(&id).is_err());
}

#[test]
fn presence_is_known_without_reading_and_forgetting_twice_is_fine() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let id = CredentialId::new(
        "product-mission-control.synthetic-test",
        format!("presence-{nonce}"),
    )
    .unwrap_or_else(|error| panic!("credential ID failed: {error}"));
    let store = WindowsCredentialStore::open()
        .unwrap_or_else(|error| panic!("credential store failed: {error}"));

    assert_eq!(store.exists(&id), Ok(false));
    store
        .set(&id, b"public-safe-synthetic-credential")
        .unwrap_or_else(|error| panic!("credential write failed: {error}"));
    let mut cleanup = CredentialCleanup {
        store: &store,
        id: &id,
        armed: true,
    };
    assert_eq!(store.exists(&id), Ok(true));
    store
        .delete(&id)
        .unwrap_or_else(|error| panic!("credential delete failed: {error}"));
    cleanup.disarm();
    assert_eq!(store.exists(&id), Ok(false));
    // Nothing left to delete is not a failure.
    assert_eq!(store.delete(&id), Ok(()));
}
