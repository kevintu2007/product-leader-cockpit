use std::collections::HashMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::sync::{Arc, Mutex};

use keyring_core::api::CredentialStore;
use keyring_core::Entry;
use windows_native_keyring_store::Store;
use zeroize::Zeroizing;

const MAX_CREDENTIAL_IDENTIFIER_LENGTH: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialId {
    service: String,
    account: String,
}

impl CredentialId {
    pub fn new(
        service: impl Into<String>,
        account: impl Into<String>,
    ) -> Result<Self, CredentialError> {
        let service = service.into();
        let account = account.into();
        validate_identifier(&service)?;
        validate_identifier(&account)?;
        Ok(Self { service, account })
    }
}

pub struct SecretBytes(Zeroizing<Vec<u8>>);

impl SecretBytes {
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        self.0.as_slice()
    }
}

pub struct WindowsCredentialStore {
    store: Arc<CredentialStore>,
    writer: Mutex<()>,
}

impl WindowsCredentialStore {
    pub fn open() -> Result<Self, CredentialError> {
        let mut configuration = HashMap::new();
        configuration.insert("prefix", "product-mission-control:");
        configuration.insert("divider", "|");
        configuration.insert("service_no_divider", "true");
        let store = Store::new_with_configuration(&configuration)
            .map_err(|_| CredentialError::Unavailable)?;
        Ok(Self {
            store,
            writer: Mutex::new(()),
        })
    }

    pub fn set(&self, id: &CredentialId, secret: &[u8]) -> Result<(), CredentialError> {
        if secret.is_empty() {
            return Err(CredentialError::InvalidSecret);
        }
        let _writer = self
            .writer
            .lock()
            .map_err(|_| CredentialError::Unavailable)?;
        self.entry(id)?
            .set_secret(secret)
            .map_err(|_| CredentialError::WriteFailed)
    }

    pub fn get(&self, id: &CredentialId) -> Result<SecretBytes, CredentialError> {
        let _writer = self
            .writer
            .lock()
            .map_err(|_| CredentialError::Unavailable)?;
        self.entry(id)?
            .get_secret()
            .map(Zeroizing::new)
            .map(SecretBytes)
            .map_err(|_| CredentialError::ReadFailed)
    }

    /// Remove the credential. One that is not there is already removed, so
    /// that is success; any other failure is reported.
    pub fn delete(&self, id: &CredentialId) -> Result<(), CredentialError> {
        let _writer = self
            .writer
            .lock()
            .map_err(|_| CredentialError::Unavailable)?;
        match self.entry(id)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(_) => Err(CredentialError::DeleteFailed),
        }
    }

    /// Whether a credential is stored, without handing its secret to the
    /// caller.
    pub fn exists(&self, id: &CredentialId) -> Result<bool, CredentialError> {
        let _writer = self
            .writer
            .lock()
            .map_err(|_| CredentialError::Unavailable)?;
        match self.entry(id)?.get_attributes() {
            Ok(_) => Ok(true),
            Err(keyring_core::Error::NoEntry) => Ok(false),
            Err(_) => Err(CredentialError::ReadFailed),
        }
    }

    fn entry(&self, id: &CredentialId) -> Result<Entry, CredentialError> {
        let mut modifiers = HashMap::new();
        modifiers.insert("persistence", "Local");
        self.store
            .build(&id.service, &id.account, Some(&modifiers))
            .map_err(|_| CredentialError::InvalidIdentifier)
    }
}

fn validate_identifier(value: &str) -> Result<(), CredentialError> {
    if value.is_empty()
        || value.len() > MAX_CREDENTIAL_IDENTIFIER_LENGTH
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(CredentialError::InvalidIdentifier);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CredentialError {
    DeleteFailed,
    InvalidIdentifier,
    InvalidSecret,
    ReadFailed,
    Unavailable,
    WriteFailed,
}

impl Display for CredentialError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DeleteFailed => "credential deletion failed",
            Self::InvalidIdentifier => "credential identifier is invalid",
            Self::InvalidSecret => "credential secret is invalid",
            Self::ReadFailed => "credential read failed",
            Self::Unavailable => "OS credential storage is unavailable",
            Self::WriteFailed => "credential write failed",
        })
    }
}

impl Error for CredentialError {}
