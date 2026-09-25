use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use atomic_write_file::AtomicWriteFile;

/// Stored in [`DisplaySettings::locale`] to mean "follow the operating
/// system's language". `und` is BCP 47 for "undetermined", so format v1
/// holds it without a new field; it is never a language to display in.
pub const FOLLOW_SYSTEM_LOCALE: &str = "und";

const SETTINGS_FILE_NAME: &str = "settings-v1.json";

/// Only what [`peek_locale`] needs from the document on disk.
#[derive(Deserialize)]
struct LocaleOnly {
    display: DisplayLocaleOnly,
}

#[derive(Deserialize)]
struct DisplayLocaleOnly {
    locale: String,
}

/// The display locale the settings document holds right now, read as the
/// file is: no store, no initialisation of a missing document, no setting
/// aside of an invalid one. `None` when there is no readable document. For a
/// reader that must not write — the second launch's single-instance message.
#[must_use]
pub fn peek_locale(root: &ProtectedSettingsRoot) -> Option<String> {
    let bytes = fs::read(root.join(SETTINGS_FILE_NAME)).ok()?;
    serde_json::from_slice::<LocaleOnly>(&bytes)
        .ok()
        .map(|document| document.display.locale)
}
/// Format 2 adds `operational.live_vault_root` (item ⑦; product owner
/// 2026-09-21 and the accepted Vault-root amendment). A format-1 document is
/// upgraded in place when it is opened: the new field starts absent, which
/// is exactly what a workspace with no configured Vault means.
///
/// Format 3 adds `operational.selected_workspace` (item ⑨; the accepted
/// sample-workspace amendment §2). An older document was written by a PMC
/// that only ever opened Live, so it upgrades to **Live** — an existing
/// profile never sees the first-run choice.
const CURRENT_FORMAT_VERSION: u32 = 3;
/// The first format that records which workspace opens.
const SELECTED_WORKSPACE_FORMAT_VERSION: u32 = 3;
const FIRST_SUPPORTED_FORMAT_VERSION: u32 = 1;
const MAX_IDENTIFIER_LENGTH: usize = 128;
const MAX_LOCALE_LENGTH: usize = 35;
const MAX_ROUTE_LENGTH: usize = 256;
const MAX_SIDEBAR_WIDTH: u16 = 4_096;
const MAX_TIMEZONE_LENGTH: usize = 64;
const MAX_WINDOW_DIMENSION: u32 = 32_768;

static WRITER_REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<WriterState>>>>> =
    OnceLock::new();

#[derive(Deserialize)]
struct VersionEnvelope {
    format_version: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectedSettingsRoot(PathBuf);

impl ProtectedSettingsRoot {
    pub fn prepare(application_directory: &str) -> Result<Self, SettingsError> {
        if application_directory.is_empty()
            || application_directory.len() > MAX_IDENTIFIER_LENGTH
            || matches!(application_directory, "." | "..")
            || !application_directory.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            })
        {
            return Err(SettingsError::InvalidApplicationDirectory);
        }
        let base = os_app_data_base()?;
        fs::create_dir_all(&base).map_err(SettingsError::Io)?;
        let canonical_base = fs::canonicalize(&base).map_err(SettingsError::Io)?;
        let root = canonical_base.join(application_directory);
        create_direct_protected_directory(&canonical_base, &root)?;
        let canonical_root = verify_direct_directory(&canonical_base, &root)?;
        Ok(Self(canonical_root))
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for ProtectedSettingsRoot {
    fn as_ref(&self) -> &Path {
        self.path()
    }
}

impl std::ops::Deref for ProtectedSettingsRoot {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        self.path()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsDocument {
    pub format_version: u32,
    pub revision: u64,
    pub display: DisplaySettings,
    pub operational: OperationalSettings,
    pub presentation: PresentationSettings,
}

/// The settings an Operational Backup carries (ADR 0010 §3): portable,
/// non-secret preferences only. No path (they belong to one machine) and no
/// presentation state; a named allow-list, so a field added to the settings
/// later is not swept into backups unreviewed.
#[derive(Serialize)]
struct BackupSettingsExport<'a> {
    format: &'static str,
    locale: &'a str,
    timezone: &'a str,
    theme: Theme,
    retention_days: u16,
    external_ai_enabled: bool,
    log_level: LogLevel,
}

impl SettingsDocument {
    /// The JSON an Operational Backup stores as `settings.json`.
    pub fn backup_export(&self) -> Result<Vec<u8>, SettingsError> {
        serde_json::to_vec(&BackupSettingsExport {
            format: "pmc-backup-settings/v1",
            locale: &self.display.locale,
            timezone: &self.display.timezone,
            theme: self.display.theme,
            retention_days: self.operational.retention_days,
            external_ai_enabled: self.operational.external_ai_enabled,
            log_level: self.operational.log_level,
        })
        .map_err(SettingsError::InvalidJson)
    }
}

/// A settings document as JSON — the preimage a restore keeps so it can put
/// the settings back.
pub fn document_json(document: &SettingsDocument) -> Result<Vec<u8>, SettingsError> {
    serde_json::to_vec(document).map_err(SettingsError::InvalidJson)
}

/// A settings document read back from [`document_json`], validated.
///
/// An older supported format is upgraded in memory, exactly as opening one
/// does: a restore prepared by an earlier PMC keeps a preimage in that
/// version, and refusing to read it after an upgrade would turn a recoverable
/// restore into an unrecoverable one.
pub fn document_from_json(bytes: &[u8]) -> Result<SettingsDocument, SettingsError> {
    let mut document: SettingsDocument =
        serde_json::from_slice(bytes).map_err(SettingsError::InvalidJson)?;
    if document.format_version >= FIRST_SUPPORTED_FORMAT_VERSION
        && document.format_version < CURRENT_FORMAT_VERSION
    {
        upgrade_in_memory(&mut document);
    }
    validate_document(&document)?;
    Ok(document)
}

/// An older supported document brought to the current format: each field it
/// lacks takes the value its absence meant when it was written.
fn upgrade_in_memory(document: &mut SettingsDocument) {
    if document.format_version < SELECTED_WORKSPACE_FORMAT_VERSION {
        // Written by a PMC that only ever opened Live.
        document.operational.selected_workspace = Some(SelectedWorkspace::Live);
    }
    document.format_version = CURRENT_FORMAT_VERSION;
}

/// Which workspace opens (the accepted sample-workspace amendment §2): the
/// person's own (**Live**) or the sample data (**Training** — the sample
/// workspace is Training made visible, never a third kind).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SelectedWorkspace {
    Live,
    Training,
}

impl SelectedWorkspace {
    #[must_use]
    pub const fn kind(self) -> crate::workspace::WorkspaceKind {
        match self {
            Self::Live => crate::workspace::WorkspaceKind::Live,
            Self::Training => crate::workspace::WorkspaceKind::Training,
        }
    }
}

/// What startup does before anything else opens (§2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupSelection {
    /// Open this workspace.
    Open(SelectedWorkspace),
    /// A genuinely new profile: show the first-run choice, and open no
    /// Ledger, Vault or backup work until it is made.
    Choose,
}

/// Decide which workspace opens, from how the settings document loaded,
/// what it records, and whether a Live workspace already exists on disk.
///
/// Only a profile that has never been used asks. Settings PMC could not read
/// (`InvalidPreserved`) never ask and never lead to sample data: they open
/// Live, and the failure is reported as before. A recorded choice is kept;
/// "not chosen yet" with a Live workspace already on disk is Live, because
/// that person has used PMC.
#[must_use]
pub fn startup_selection(
    disposition: &LoadDisposition,
    recorded: Option<SelectedWorkspace>,
    live_exists: bool,
) -> StartupSelection {
    match disposition {
        LoadDisposition::InvalidPreserved { .. } => StartupSelection::Open(SelectedWorkspace::Live),
        LoadDisposition::Loaded | LoadDisposition::Upgraded | LoadDisposition::MissingDefaulted => {
            match recorded {
                Some(chosen) => StartupSelection::Open(chosen),
                None if live_exists => StartupSelection::Open(SelectedWorkspace::Live),
                None => StartupSelection::Choose,
            }
        }
    }
}

/// The settings carried by an Operational Backup, read back for a restore
/// (S7-B1; DG3 restore amendment §3.4): exactly the six fields of the export,
/// every one validated as the settings document validates it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BackupSettings {
    format: String,
    pub locale: String,
    pub timezone: String,
    pub theme: Theme,
    pub retention_days: u16,
    pub external_ai_enabled: bool,
    pub log_level: LogLevel,
}

impl BackupSettings {
    /// Parse and validate an archive's `settings.json`.
    pub fn parse(bytes: &[u8]) -> Result<Self, SettingsError> {
        let settings: Self = serde_json::from_slice(bytes).map_err(SettingsError::InvalidJson)?;
        if settings.format != "pmc-backup-settings/v1" {
            return Err(SettingsError::InvalidContent {
                field: "format",
                code: "unsupported",
            });
        }
        let mut probe = SettingsDocument::default();
        settings.apply_to(&mut probe);
        validate_document(&probe)?;
        Ok(settings)
    }

    fn apply_to(&self, document: &mut SettingsDocument) {
        document.display.locale.clone_from(&self.locale);
        document.display.timezone.clone_from(&self.timezone);
        document.display.theme = self.theme;
        document.operational.retention_days = self.retention_days;
        document.operational.external_ai_enabled = self.external_ai_enabled;
        document.operational.log_level = self.log_level;
    }
}

impl Default for SettingsDocument {
    fn default() -> Self {
        Self {
            format_version: CURRENT_FORMAT_VERSION,
            revision: 0,
            display: DisplaySettings::default(),
            operational: OperationalSettings::default(),
            presentation: PresentationSettings::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DisplaySettings {
    pub locale: String,
    pub timezone: String,
    pub theme: Theme,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            locale: "zh-TW".to_owned(),
            timezone: "Asia/Taipei".to_owned(),
            theme: Theme::System,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationalSettings {
    pub authority_root: Option<CanonicalDirectoryPath>,
    pub backup_destination: Option<DeferredDirectoryPath>,
    /// The Live workspace's Product Vault folder, chosen by the person
    /// (item ⑦). Deferred, like the backup folder: it may sit on a drive
    /// that is not plugged in, and settings must still load. `None` means no
    /// Vault is configured — never "the Vault is missing". Training derives
    /// its own synthetic Vault and ignores this.
    ///
    /// `default` so a format-1 document upgrades by gaining it as absent.
    #[serde(default)]
    pub live_vault_root: Option<DeferredDirectoryPath>,
    /// Which workspace opens (item ⑨). `None` only for a new profile that
    /// has not chosen yet; an older document upgrades to Live. Not part of a
    /// backup, so restoring one cannot switch workspaces.
    #[serde(default)]
    pub selected_workspace: Option<SelectedWorkspace>,
    pub retention_days: u16,
    pub external_ai_enabled: bool,
    pub log_level: LogLevel,
}

impl Default for OperationalSettings {
    fn default() -> Self {
        Self {
            authority_root: None,
            backup_destination: None,
            live_vault_root: None,
            selected_workspace: None,
            retention_days: 30,
            external_ai_enabled: false,
            log_level: LogLevel::Info,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PresentationSettings {
    pub last_route: Option<String>,
    pub sidebar_width: Option<u16>,
    pub window: Option<WindowGeometry>,
    pub table_preferences: Vec<TablePreference>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OperationalPatch {
    pub authority_root: ValuePatch<CanonicalDirectoryPath>,
    pub backup_destination: ValuePatch<DeferredDirectoryPath>,
    pub live_vault_root: ValuePatch<DeferredDirectoryPath>,
    pub retention_days: Option<u16>,
    pub external_ai_enabled: Option<bool>,
    pub log_level: Option<LogLevel>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PresentationPatch {
    pub last_route: ValuePatch<String>,
    pub sidebar_width: ValuePatch<u16>,
    pub window: ValuePatch<WindowGeometry>,
    pub table_preferences: Option<Vec<TablePreference>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ValuePatch<T> {
    Clear,
    Set(T),
    #[default]
    Unchanged,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct CanonicalDirectoryPath(PathBuf);

impl CanonicalDirectoryPath {
    pub fn new(path: PathBuf) -> Result<Self, SettingsError> {
        if !path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
        {
            return Err(SettingsError::InvalidDirectoryPath);
        }
        let normalized = path.components().collect::<PathBuf>();
        if normalized != path {
            return Err(SettingsError::InvalidDirectoryPath);
        }
        ensure_directory_path_has_no_links(&path)?;
        let metadata = fs::metadata(&path).map_err(|_| SettingsError::InvalidDirectoryPath)?;
        if !metadata.is_dir() {
            return Err(SettingsError::InvalidDirectoryPath);
        }
        let canonical = fs::canonicalize(&path).map_err(|_| SettingsError::InvalidDirectoryPath)?;
        ensure_directory_path_has_no_links(&canonical)?;
        Ok(Self(canonical))
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// Whether this folder is `other`, lies inside it, or contains it.
    /// Compared by path components, never by string prefix, so `D:\Vault2`
    /// does not sit inside `D:\Vault`. Used to keep a Vault away from the
    /// folders PMC already owns (item ⑦, DG3 Vault-root amendment §2).
    #[must_use]
    pub fn overlaps(&self, other: &Path) -> bool {
        self.0.starts_with(other) || other.starts_with(&self.0)
    }
}

impl<'de> Deserialize<'de> for CanonicalDirectoryPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let path = PathBuf::deserialize(deserializer)?;
        Self::new(path).map_err(|_| serde::de::Error::custom("invalid canonical directory path"))
    }
}

/// A folder stored in settings that may legitimately be absent when settings
/// load — the backup destination on a drive that is not plugged in. Loading
/// checks only that the path is absolute and normalized; the filesystem is
/// checked by [`DeferredDirectoryPath::revalidate`] every time it is used.
/// Unlike [`CanonicalDirectoryPath`], holding one proves nothing about the
/// filesystem.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct DeferredDirectoryPath(PathBuf);

impl DeferredDirectoryPath {
    /// Prove, now, that the folder exists, is a directory and is reached
    /// through no link.
    pub fn revalidate(&self) -> Result<CanonicalDirectoryPath, SettingsError> {
        CanonicalDirectoryPath::new(self.0.clone())
    }

    /// The stored path itself, for a caller inside the host that must reach
    /// the folder (it never crosses IPC). Proves nothing about the
    /// filesystem: use [`Self::revalidate`] before touching it.
    #[must_use]
    pub fn as_stored(&self) -> &Path {
        &self.0
    }

    /// The folder's own name — its last component only — so a person can
    /// recognise it; the rest of the path never leaves the host (ADR 0011,
    /// DG3 backup-setup amendment §2).
    ///
    /// A drive root has no last component, so it is named by its drive
    /// (`D:`), which is all of it and no more than the person chose.
    /// `None` only when the path has neither, which a stored absolute path
    /// cannot.
    #[must_use]
    pub fn folder_name(&self) -> Option<String> {
        self.0
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .or_else(|| {
                self.0.components().next().and_then(|component| {
                    matches!(component, std::path::Component::Prefix(_)).then(|| {
                        component
                            .as_os_str()
                            .to_string_lossy()
                            .trim_end_matches(['\\', '/'])
                            .to_owned()
                    })
                })
            })
    }

    fn has_stored_form(path: &Path) -> bool {
        path.is_absolute()
            && !path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir | std::path::Component::ParentDir
                )
            })
            && path.components().collect::<PathBuf>() == path
    }
}

impl From<CanonicalDirectoryPath> for DeferredDirectoryPath {
    fn from(directory: CanonicalDirectoryPath) -> Self {
        Self(directory.0)
    }
}

impl<'de> Deserialize<'de> for DeferredDirectoryPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let path = PathBuf::deserialize(deserializer)?;
        if Self::has_stored_form(&path) {
            Ok(Self(path))
        } else {
            Err(serde::de::Error::custom("invalid stored directory path"))
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DisplayPatch {
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub theme: Option<Theme>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingsPatch {
    Display(DisplayPatch),
    Operational(OperationalPatch),
    Presentation(PresentationPatch),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PatchOutcome {
    Committed { revision: u64 },
    Stale { current_revision: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitializeOutcome {
    AlreadyInitialized { current_revision: u64 },
    Initialized { revision: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QueueOutcome {
    Queued { base_revision: u64 },
    Stale { current_revision: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlushOutcome {
    Committed { revision: u64 },
    NoPending,
    StaleDiscarded { current_revision: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelOutcome {
    Cancelled { base_revision: u64 },
    NoPending,
    RevisionMismatch { pending_base_revision: u64 },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TablePreference {
    pub table_id: String,
    pub density: TableDensity,
    pub visible_columns: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TableDensity {
    Compact,
    Comfortable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadDisposition {
    Loaded,
    /// Loaded from an older supported format and written back at the current
    /// one. Everything the person set is kept, and the revision did not
    /// move; the caller is told because an upgrade in place is worth
    /// recording, not because anything is wrong.
    Upgraded,
    MissingDefaulted,
    InvalidPreserved {
        path: PathBuf,
        reason: InvalidSettingsReason,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidSettingsReason {
    Malformed,
    Schema,
    UnsupportedFormatVersion { found: u32, supported: u32 },
}

#[derive(Debug)]
pub struct OpenedSettings {
    store: SettingsStore,
    document: SettingsDocument,
    disposition: LoadDisposition,
}

impl OpenedSettings {
    #[must_use]
    pub const fn document(&self) -> &SettingsDocument {
        &self.document
    }

    #[must_use]
    pub fn disposition(&self) -> LoadDisposition {
        self.disposition.clone()
    }

    #[must_use]
    pub const fn store(&self) -> &SettingsStore {
        &self.store
    }

    /// The store alone, for a caller that keeps it for the process lifetime
    /// and reads the current document through [`SettingsStore::read`].
    #[must_use]
    pub fn into_store(self) -> SettingsStore {
        self.store
    }
}

#[derive(Debug)]
pub struct SettingsStore {
    settings_file: PathBuf,
    writer: Arc<Mutex<WriterState>>,
}

#[derive(Debug, Default)]
struct WriterState {
    pending_presentation: Option<PendingPresentation>,
}

#[derive(Clone, Debug)]
struct PendingPresentation {
    base_revision: u64,
    patch: PresentationPatch,
}

impl SettingsStore {
    pub fn open(root: &ProtectedSettingsRoot) -> Result<OpenedSettings, SettingsError> {
        let settings_file = root.join(SETTINGS_FILE_NAME);
        let store = Self {
            writer: writer_for(&settings_file)?,
            settings_file,
        };
        match fs::read(&store.settings_file) {
            Ok(bytes) => {
                let envelope = match serde_json::from_slice::<VersionEnvelope>(&bytes) {
                    Ok(envelope) => envelope,
                    Err(error) => {
                        return store.fallback_after_invalid(classify_json_error(&error));
                    }
                };
                if envelope.format_version < FIRST_SUPPORTED_FORMAT_VERSION
                    || envelope.format_version > CURRENT_FORMAT_VERSION
                {
                    return store.fallback_after_invalid(
                        InvalidSettingsReason::UnsupportedFormatVersion {
                            found: envelope.format_version,
                            supported: CURRENT_FORMAT_VERSION,
                        },
                    );
                }
                // An older supported format gains the fields it lacks (each
                // absent, which is their meaning) and is written back at the
                // current version before anything reads it. The revision
                // does not move: nothing the person set changed.
                if envelope.format_version < CURRENT_FORMAT_VERSION {
                    return store.upgrade_format();
                }
                match serde_json::from_slice::<SettingsDocument>(&bytes) {
                    Ok(document) => match validate_document(&document) {
                        Ok(()) => Ok(OpenedSettings {
                            store,
                            document,
                            disposition: LoadDisposition::Loaded,
                        }),
                        Err(SettingsError::InvalidContent { .. }) => {
                            store.fallback_after_invalid(InvalidSettingsReason::Schema)
                        }
                        Err(error) => Err(error),
                    },
                    Err(error) => store.fallback_after_invalid(classify_json_error(&error)),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(OpenedSettings {
                store,
                document: SettingsDocument::default(),
                disposition: LoadDisposition::MissingDefaulted,
            }),
            Err(error) => Err(SettingsError::Io(error)),
        }
    }

    pub fn initialize(&self) -> Result<InitializeOutcome, SettingsError> {
        self.initialize_document(SettingsDocument::default())
    }

    /// First run with a chosen display locale: the whole revision-0 document
    /// in one atomic write, so no crash can leave the default locale behind
    /// in its place. Every other setting is the default, as with
    /// [`Self::initialize`], and an existing document is never overwritten.
    pub fn initialize_with_locale(&self, locale: &str) -> Result<InitializeOutcome, SettingsError> {
        let mut document = SettingsDocument::default();
        locale.clone_into(&mut document.display.locale);
        self.initialize_document(document)
    }

    /// First run with a chosen locale and what is known about the workspace:
    /// `Some(Live)` for a profile that has used PMC before (or whose
    /// settings could not be read), `None` for a new profile that has not
    /// chosen yet. One atomic write; an existing document is never
    /// overwritten.
    pub fn initialize_first_run(
        &self,
        locale: &str,
        selected_workspace: Option<SelectedWorkspace>,
    ) -> Result<InitializeOutcome, SettingsError> {
        let mut document = SettingsDocument::default();
        locale.clone_into(&mut document.display.locale);
        document.operational.selected_workspace = selected_workspace;
        self.initialize_document(document)
    }

    /// Record which workspace opens next (first-run choice or a switch in
    /// Settings), against the revision the caller read. There is no way back
    /// to "not chosen".
    pub fn select_workspace(
        &self,
        expected_revision: u64,
        selected: SelectedWorkspace,
    ) -> Result<PatchOutcome, SettingsError> {
        self.change_document(expected_revision, |document| {
            document.operational.selected_workspace = Some(selected);
        })
    }

    /// The document as it is on disk now.
    pub fn read(&self) -> Result<SettingsDocument, SettingsError> {
        self.read_document()
    }

    fn initialize_document(
        &self,
        document: SettingsDocument,
    ) -> Result<InitializeOutcome, SettingsError> {
        validate_document(&document)?;
        let _writer = self
            .writer
            .lock()
            .map_err(|_| SettingsError::WriterPoisoned)?;
        if self.settings_file.exists() {
            let current = self.read_document()?;
            return Ok(InitializeOutcome::AlreadyInitialized {
                current_revision: current.revision,
            });
        }
        self.write_atomic(&document)?;
        Ok(InitializeOutcome::Initialized {
            revision: document.revision,
        })
    }

    pub fn apply_durable(
        &self,
        expected_revision: u64,
        patch: SettingsPatch,
    ) -> Result<PatchOutcome, SettingsError> {
        self.apply_durable_with_lock_hook(expected_revision, patch, || {})
    }

    /// A restore's settings: all six archived fields in one atomic write —
    /// never display and operational separately, which a crash could leave
    /// half done. Nothing else changes: not the Vault root, not the backup
    /// folder, not presentation state.
    pub fn apply_restore_export(
        &self,
        expected_revision: u64,
        settings: &BackupSettings,
    ) -> Result<PatchOutcome, SettingsError> {
        self.change_document(expected_revision, |document| settings.apply_to(document))
    }

    /// Put back the settings a restore replaced (its rollback): every field of
    /// `preimage` except the revision, which moves forward, and the selected
    /// workspace, which a restore never changes (sample-workspace amendment
    /// §2) and so its rollback must not either.
    pub fn put_back(
        &self,
        expected_revision: u64,
        preimage: &SettingsDocument,
    ) -> Result<PatchOutcome, SettingsError> {
        self.change_document(expected_revision, |document| {
            let revision = document.revision;
            let selected = document.operational.selected_workspace;
            *document = preimage.clone();
            document.revision = revision;
            document.operational.selected_workspace = selected;
        })
    }

    fn change_document(
        &self,
        expected_revision: u64,
        change: impl FnOnce(&mut SettingsDocument),
    ) -> Result<PatchOutcome, SettingsError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| SettingsError::WriterPoisoned)?;
        let bytes = fs::read(&self.settings_file).map_err(SettingsError::Io)?;
        let mut document = serde_json::from_slice::<SettingsDocument>(&bytes)
            .map_err(SettingsError::InvalidJson)?;
        validate_document(&document)?;
        if document.revision != expected_revision {
            return Ok(PatchOutcome::Stale {
                current_revision: document.revision,
            });
        }
        change(&mut document);
        validate_document(&document)?;
        document.revision = document
            .revision
            .checked_add(1)
            .ok_or(SettingsError::RevisionExhausted)?;
        self.write_atomic(&document)?;
        // A queued presentation write was based on the document just
        // replaced; it must not land on top of the new one.
        writer.pending_presentation = None;
        Ok(PatchOutcome::Committed {
            revision: document.revision,
        })
    }

    fn apply_durable_with_lock_hook(
        &self,
        expected_revision: u64,
        patch: SettingsPatch,
        lock_hook: impl FnOnce(),
    ) -> Result<PatchOutcome, SettingsError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| SettingsError::WriterPoisoned)?;
        lock_hook();
        let bytes = fs::read(&self.settings_file).map_err(SettingsError::Io)?;
        let mut document = serde_json::from_slice::<SettingsDocument>(&bytes)
            .map_err(SettingsError::InvalidJson)?;
        validate_document(&document)?;
        if document.revision != expected_revision {
            return Ok(PatchOutcome::Stale {
                current_revision: document.revision,
            });
        }

        apply_patch(&mut document, patch);
        validate_document(&document)?;
        document.revision = document
            .revision
            .checked_add(1)
            .ok_or(SettingsError::RevisionExhausted)?;
        self.write_atomic(&document)?;
        if let Some(pending) = writer.pending_presentation.as_mut() {
            if pending.base_revision == expected_revision {
                pending.base_revision = document.revision;
            } else {
                writer.pending_presentation = None;
            }
        }
        Ok(PatchOutcome::Committed {
            revision: document.revision,
        })
    }

    pub fn queue_presentation(
        &self,
        expected_revision: u64,
        patch: PresentationPatch,
    ) -> Result<QueueOutcome, SettingsError> {
        self.queue_presentation_with_attempt_hook(expected_revision, patch, || {})
    }

    fn queue_presentation_with_attempt_hook(
        &self,
        expected_revision: u64,
        patch: PresentationPatch,
        attempt_hook: impl FnOnce(),
    ) -> Result<QueueOutcome, SettingsError> {
        attempt_hook();
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| SettingsError::WriterPoisoned)?;
        let document = self.read_document()?;
        if document.revision != expected_revision {
            return Ok(QueueOutcome::Stale {
                current_revision: document.revision,
            });
        }

        match writer.pending_presentation.as_mut() {
            Some(pending) if pending.base_revision == expected_revision => {
                merge_presentation_patch(&mut pending.patch, patch);
            }
            _ => {
                writer.pending_presentation = Some(PendingPresentation {
                    base_revision: expected_revision,
                    patch,
                });
            }
        }
        Ok(QueueOutcome::Queued {
            base_revision: expected_revision,
        })
    }

    pub fn flush_pending_presentation(&self) -> Result<FlushOutcome, SettingsError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| SettingsError::WriterPoisoned)?;
        let Some(pending) = writer.pending_presentation.clone() else {
            return Ok(FlushOutcome::NoPending);
        };
        let mut document = self.read_document()?;
        if document.revision != pending.base_revision {
            writer.pending_presentation = None;
            return Ok(FlushOutcome::StaleDiscarded {
                current_revision: document.revision,
            });
        }

        apply_presentation_patch(&mut document.presentation, pending.patch);
        validate_document(&document)?;
        document.revision = document
            .revision
            .checked_add(1)
            .ok_or(SettingsError::RevisionExhausted)?;
        self.write_atomic(&document)?;
        writer.pending_presentation = None;
        Ok(FlushOutcome::Committed {
            revision: document.revision,
        })
    }

    pub fn cancel_pending_presentation(
        &self,
        expected_base_revision: u64,
    ) -> Result<CancelOutcome, SettingsError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| SettingsError::WriterPoisoned)?;
        let Some(pending) = writer.pending_presentation.as_ref() else {
            return Ok(CancelOutcome::NoPending);
        };
        if pending.base_revision != expected_base_revision {
            return Ok(CancelOutcome::RevisionMismatch {
                pending_base_revision: pending.base_revision,
            });
        }
        let base_revision = pending.base_revision;
        writer.pending_presentation = None;
        Ok(CancelOutcome::Cancelled { base_revision })
    }

    fn read_document(&self) -> Result<SettingsDocument, SettingsError> {
        let bytes = fs::read(&self.settings_file).map_err(SettingsError::Io)?;
        let document = serde_json::from_slice(&bytes).map_err(SettingsError::InvalidJson)?;
        validate_document(&document)?;
        Ok(document)
    }

    fn write_atomic(&self, document: &SettingsDocument) -> Result<(), SettingsError> {
        if let Some(parent) = self.settings_file.parent() {
            fs::create_dir_all(parent).map_err(SettingsError::Io)?;
        }
        let bytes = serde_json::to_vec_pretty(document).map_err(SettingsError::InvalidJson)?;
        let mut file = AtomicWriteFile::open(&self.settings_file).map_err(SettingsError::Io)?;
        file.write_all(&bytes).map_err(SettingsError::Io)?;
        file.commit().map_err(SettingsError::Io)
    }

    fn preserve_invalid_file(&self) -> Result<PathBuf, SettingsError> {
        let root = self
            .settings_file
            .parent()
            .ok_or(SettingsError::MissingSettingsRoot)?;
        let diagnostics = root.join("diagnostics");
        create_direct_protected_directory(root, &diagnostics)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());

        for suffix in 0_u16..=u16::MAX {
            let destination =
                diagnostics.join(format!("settings-invalid-{timestamp}-{suffix}.json"));
            if destination.exists() {
                continue;
            }
            verify_direct_directory(root, &diagnostics)?;
            match fs::rename(&self.settings_file, &destination) {
                Ok(()) => return Ok(destination),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(SettingsError::Io(error)),
            }
        }

        Err(SettingsError::DiagnosticNameExhausted)
    }

    /// Write an older supported document back at the current format, under
    /// the same writer lock every other write takes, re-reading inside it so
    /// a change committed between the first read and the lock is upgraded
    /// rather than overwritten by the copy this call started with.
    ///
    /// A write that fails is reported as the I/O failure it is. It is never
    /// treated as an invalid document: a perfectly good settings file on a
    /// read-only disk must not be set aside as corrupt.
    fn upgrade_format(self) -> Result<OpenedSettings, SettingsError> {
        let document = {
            let _writer = self
                .writer
                .lock()
                .map_err(|_| SettingsError::WriterPoisoned)?;
            let bytes = fs::read(&self.settings_file).map_err(SettingsError::Io)?;
            let envelope = match serde_json::from_slice::<VersionEnvelope>(&bytes) {
                Ok(envelope) => envelope,
                Err(error) => {
                    drop(_writer);
                    return self.fallback_after_invalid(classify_json_error(&error));
                }
            };
            if envelope.format_version < FIRST_SUPPORTED_FORMAT_VERSION
                || envelope.format_version > CURRENT_FORMAT_VERSION
            {
                drop(_writer);
                return self.fallback_after_invalid(
                    InvalidSettingsReason::UnsupportedFormatVersion {
                        found: envelope.format_version,
                        supported: CURRENT_FORMAT_VERSION,
                    },
                );
            }
            let mut document = match serde_json::from_slice::<SettingsDocument>(&bytes) {
                Ok(document) => document,
                Err(error) => {
                    drop(_writer);
                    return self.fallback_after_invalid(classify_json_error(&error));
                }
            };
            upgrade_in_memory(&mut document);
            match validate_document(&document) {
                Ok(()) => {}
                Err(SettingsError::InvalidContent { .. }) => {
                    drop(_writer);
                    return self.fallback_after_invalid(InvalidSettingsReason::Schema);
                }
                Err(error) => return Err(error),
            }
            // Already current: another process upgraded it while we waited.
            if envelope.format_version < CURRENT_FORMAT_VERSION {
                self.write_atomic(&document)?;
            }
            document
        };
        Ok(OpenedSettings {
            store: self,
            document,
            disposition: LoadDisposition::Upgraded,
        })
    }

    fn fallback_after_invalid(
        self,
        reason: InvalidSettingsReason,
    ) -> Result<OpenedSettings, SettingsError> {
        let path = self.preserve_invalid_file()?;
        Ok(OpenedSettings {
            store: self,
            document: SettingsDocument::default(),
            disposition: LoadDisposition::InvalidPreserved { path, reason },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;

    use super::*;

    #[test]
    fn presentation_ready_during_durable_write_linearizes_after_the_commit() {
        let root_path = std::env::temp_dir().join(format!(
            "pmc-settings-ready-during-durable-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_else(|error| panic!("system clock is before the epoch: {error}"))
                .as_nanos()
        ));
        fs::create_dir(&root_path)
            .unwrap_or_else(|error| panic!("fixture directory creation failed: {error}"));
        let root = ProtectedSettingsRoot(root_path.clone());
        let durable = SettingsStore::open(&root)
            .unwrap_or_else(|error| panic!("durable store open failed: {error}"));
        durable
            .store()
            .initialize()
            .unwrap_or_else(|error| panic!("initialization failed: {error}"));
        let presentation = SettingsStore::open(&root)
            .unwrap_or_else(|error| panic!("presentation store open failed: {error}"));

        let (durable_locked_tx, durable_locked_rx) = mpsc::channel();
        let (release_durable_tx, release_durable_rx) = mpsc::channel();
        let durable_thread = thread::spawn(move || {
            durable.store().apply_durable_with_lock_hook(
                0,
                SettingsPatch::Operational(OperationalPatch {
                    retention_days: Some(45),
                    ..OperationalPatch::default()
                }),
                || {
                    durable_locked_tx
                        .send(())
                        .unwrap_or_else(|error| panic!("lock signal failed: {error}"));
                    release_durable_rx
                        .recv()
                        .unwrap_or_else(|error| panic!("release wait failed: {error}"));
                },
            )
        });
        durable_locked_rx
            .recv()
            .unwrap_or_else(|error| panic!("durable lock wait failed: {error}"));

        let (presentation_attempt_tx, presentation_attempt_rx) = mpsc::channel();
        let presentation_thread = thread::spawn(move || {
            presentation.store().queue_presentation_with_attempt_hook(
                0,
                PresentationPatch {
                    sidebar_width: ValuePatch::Set(320),
                    ..PresentationPatch::default()
                },
                || {
                    presentation_attempt_tx
                        .send(())
                        .unwrap_or_else(|error| panic!("attempt signal failed: {error}"));
                },
            )
        });
        presentation_attempt_rx
            .recv()
            .unwrap_or_else(|error| panic!("presentation attempt wait failed: {error}"));
        release_durable_tx
            .send(())
            .unwrap_or_else(|error| panic!("durable release failed: {error}"));

        assert_eq!(
            durable_thread
                .join()
                .unwrap_or_else(|_| panic!("durable writer panicked"))
                .unwrap_or_else(|error| panic!("durable write failed: {error}")),
            PatchOutcome::Committed { revision: 1 }
        );
        assert_eq!(
            presentation_thread
                .join()
                .unwrap_or_else(|_| panic!("presentation writer panicked"))
                .unwrap_or_else(|error| panic!("presentation queue failed: {error}")),
            QueueOutcome::Stale {
                current_revision: 1
            }
        );

        let reopened = SettingsStore::open(&root)
            .unwrap_or_else(|error| panic!("verification reopen failed: {error}"));
        assert_eq!(reopened.document().operational.retention_days, 45);
        assert_eq!(reopened.document().presentation.sidebar_width, None);
        drop(reopened);
        fs::remove_dir_all(root_path)
            .unwrap_or_else(|error| panic!("fixture cleanup failed: {error}"));
    }
}

#[cfg(target_os = "windows")]
fn os_app_data_base() -> Result<PathBuf, SettingsError> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or(SettingsError::OsAppDataUnavailable)
}

#[cfg(target_os = "macos")]
fn os_app_data_base() -> Result<PathBuf, SettingsError> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library").join("Application Support"))
        .ok_or(SettingsError::OsAppDataUnavailable)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn os_app_data_base() -> Result<PathBuf, SettingsError> {
    if let Some(xdg_data_home) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(xdg_data_home));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".local").join("share"))
        .ok_or(SettingsError::OsAppDataUnavailable)
}

fn create_direct_protected_directory(parent: &Path, path: &Path) -> Result<(), SettingsError> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(SettingsError::Io(error)),
    }
    let verified = verify_direct_directory(parent, path)?;
    set_private_directory_permissions(&verified)
}

fn verify_direct_directory(parent: &Path, path: &Path) -> Result<PathBuf, SettingsError> {
    let metadata = fs::symlink_metadata(path).map_err(SettingsError::Io)?;
    if !metadata.is_dir() || is_link_or_reparse_point(&metadata) {
        return Err(SettingsError::LinkedSettingsDirectory);
    }
    let canonical = fs::canonicalize(path).map_err(SettingsError::Io)?;
    if canonical != path || !canonical.starts_with(parent) {
        return Err(SettingsError::SettingsRootOutsideOsAppData);
    }
    Ok(canonical)
}

#[cfg(unix)]
fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn ensure_directory_path_has_no_links(path: &Path) -> Result<(), SettingsError> {
    let mut ancestors = path
        .ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .collect::<Vec<_>>();
    ancestors.reverse();
    for ancestor in ancestors {
        let metadata =
            fs::symlink_metadata(ancestor).map_err(|_| SettingsError::InvalidDirectoryPath)?;
        if is_link_or_reparse_point(&metadata) {
            return Err(SettingsError::InvalidDirectoryPath);
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<(), SettingsError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(SettingsError::Io)
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<(), SettingsError> {
    Ok(())
}

fn writer_for(settings_file: &Path) -> Result<Arc<Mutex<WriterState>>, SettingsError> {
    let registry = WRITER_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    let mut writers = registry
        .lock()
        .map_err(|_| SettingsError::WriterRegistryPoisoned)?;
    writers.retain(|_, writer| writer.strong_count() > 0);
    if let Some(writer) = writers.get(settings_file).and_then(Weak::upgrade) {
        return Ok(writer);
    }
    let writer = Arc::new(Mutex::new(WriterState::default()));
    writers.insert(settings_file.to_path_buf(), Arc::downgrade(&writer));
    Ok(writer)
}

fn classify_json_error(error: &serde_json::Error) -> InvalidSettingsReason {
    match error.classify() {
        serde_json::error::Category::Data => InvalidSettingsReason::Schema,
        serde_json::error::Category::Eof
        | serde_json::error::Category::Io
        | serde_json::error::Category::Syntax => InvalidSettingsReason::Malformed,
    }
}

fn apply_patch(document: &mut SettingsDocument, patch: SettingsPatch) {
    match patch {
        SettingsPatch::Display(patch) => {
            if let Some(locale) = patch.locale {
                document.display.locale = locale;
            }
            if let Some(timezone) = patch.timezone {
                document.display.timezone = timezone;
            }
            if let Some(theme) = patch.theme {
                document.display.theme = theme;
            }
        }
        SettingsPatch::Operational(patch) => {
            apply_value_patch(
                &mut document.operational.authority_root,
                patch.authority_root,
            );
            apply_value_patch(
                &mut document.operational.backup_destination,
                patch.backup_destination,
            );
            apply_value_patch(
                &mut document.operational.live_vault_root,
                patch.live_vault_root,
            );
            if let Some(retention_days) = patch.retention_days {
                document.operational.retention_days = retention_days;
            }
            if let Some(external_ai_enabled) = patch.external_ai_enabled {
                document.operational.external_ai_enabled = external_ai_enabled;
            }
            if let Some(log_level) = patch.log_level {
                document.operational.log_level = log_level;
            }
        }
        SettingsPatch::Presentation(patch) => {
            apply_presentation_patch(&mut document.presentation, patch);
        }
    }
}

fn apply_presentation_patch(presentation: &mut PresentationSettings, patch: PresentationPatch) {
    apply_value_patch(&mut presentation.last_route, patch.last_route);
    apply_value_patch(&mut presentation.sidebar_width, patch.sidebar_width);
    apply_value_patch(&mut presentation.window, patch.window);
    if let Some(table_preferences) = patch.table_preferences {
        presentation.table_preferences = table_preferences;
    }
}

fn merge_presentation_patch(current: &mut PresentationPatch, newer: PresentationPatch) {
    if newer.last_route != ValuePatch::Unchanged {
        current.last_route = newer.last_route;
    }
    if newer.sidebar_width != ValuePatch::Unchanged {
        current.sidebar_width = newer.sidebar_width;
    }
    if newer.window != ValuePatch::Unchanged {
        current.window = newer.window;
    }
    if newer.table_preferences.is_some() {
        current.table_preferences = newer.table_preferences;
    }
}

fn apply_value_patch<T>(target: &mut Option<T>, patch: ValuePatch<T>) {
    match patch {
        ValuePatch::Clear => *target = None,
        ValuePatch::Set(value) => *target = Some(value),
        ValuePatch::Unchanged => {}
    }
}

fn validate_document(document: &SettingsDocument) -> Result<(), SettingsError> {
    if document.format_version != CURRENT_FORMAT_VERSION {
        return Err(SettingsError::UnsupportedFormatVersion(
            document.format_version,
        ));
    }
    validate_text(
        "display.locale",
        &document.display.locale,
        MAX_LOCALE_LENGTH,
    )?;
    if document
        .display
        .locale
        .parse::<icu_locale_core::Locale>()
        .is_err()
    {
        return Err(SettingsError::InvalidContent {
            field: "display.locale",
            code: "invalid-locale",
        });
    }
    if jiff::tz::TimeZone::get(&document.display.timezone).is_err() {
        return Err(SettingsError::InvalidContent {
            field: "display.timezone",
            code: "invalid-timezone",
        });
    }
    validate_text(
        "display.timezone",
        &document.display.timezone,
        MAX_TIMEZONE_LENGTH,
    )?;
    if document.operational.retention_days == 0 {
        return Err(SettingsError::InvalidContent {
            field: "operational.retention_days",
            code: "must-be-positive",
        });
    }
    if let Some(route) = document.presentation.last_route.as_deref() {
        if route.len() > MAX_ROUTE_LENGTH
            || !route.starts_with('/')
            || route.contains("://")
            || route.contains('\\')
            || route.chars().any(char::is_control)
        {
            return Err(SettingsError::InvalidContent {
                field: "presentation.last_route",
                code: "invalid-app-route",
            });
        }
    }
    if document
        .presentation
        .sidebar_width
        .is_some_and(|width| width > MAX_SIDEBAR_WIDTH)
    {
        return Err(SettingsError::InvalidContent {
            field: "presentation.sidebar_width",
            code: "too-large",
        });
    }
    if document.presentation.window.as_ref().is_some_and(|window| {
        window.width == 0
            || window.height == 0
            || window.width > MAX_WINDOW_DIMENSION
            || window.height > MAX_WINDOW_DIMENSION
    }) {
        return Err(SettingsError::InvalidContent {
            field: "presentation.window",
            code: "invalid-dimensions",
        });
    }
    let mut table_ids = HashSet::new();
    for table in &document.presentation.table_preferences {
        validate_text(
            "presentation.table_preferences.table_id",
            &table.table_id,
            MAX_IDENTIFIER_LENGTH,
        )?;
        if !table_ids.insert(&table.table_id) {
            return Err(SettingsError::InvalidContent {
                field: "presentation.table_preferences.table_id",
                code: "duplicate",
            });
        }
        for column in &table.visible_columns {
            validate_text(
                "presentation.table_preferences.visible_columns",
                column,
                MAX_IDENTIFIER_LENGTH,
            )?;
        }
    }
    Ok(())
}

fn validate_text(
    field: &'static str,
    value: &str,
    maximum_length: usize,
) -> Result<(), SettingsError> {
    if value.is_empty() || value.len() > maximum_length || value.chars().any(char::is_control) {
        return Err(SettingsError::InvalidContent {
            field,
            code: "invalid-text",
        });
    }
    Ok(())
}

#[derive(Debug)]
pub enum SettingsError {
    DiagnosticNameExhausted,
    InvalidJson(serde_json::Error),
    InvalidApplicationDirectory,
    InvalidContent {
        field: &'static str,
        code: &'static str,
    },
    InvalidDirectoryPath,
    Io(std::io::Error),
    LinkedSettingsDirectory,
    MissingSettingsRoot,
    OsAppDataUnavailable,
    RevisionExhausted,
    SettingsRootOutsideOsAppData,
    UnsupportedFormatVersion(u32),
    WriterPoisoned,
    WriterRegistryPoisoned,
}

impl Display for SettingsError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DiagnosticNameExhausted => {
                formatter.write_str("unable to allocate invalid-settings diagnostic name")
            }
            Self::InvalidJson(error) => write!(formatter, "invalid settings document: {error}"),
            Self::InvalidApplicationDirectory => {
                formatter.write_str("invalid application settings directory")
            }
            Self::InvalidContent { field, code } => {
                write!(formatter, "invalid settings field {field}: {code}")
            }
            Self::InvalidDirectoryPath => {
                formatter.write_str("directory path must be absolute and canonical")
            }
            Self::Io(error) => write!(formatter, "settings I/O failed: {error}"),
            Self::LinkedSettingsDirectory => {
                formatter.write_str("settings directory cannot be a link or reparse point")
            }
            Self::MissingSettingsRoot => formatter.write_str("settings root is unavailable"),
            Self::OsAppDataUnavailable => {
                formatter.write_str("OS application-data directory is unavailable")
            }
            Self::RevisionExhausted => formatter.write_str("settings revision is exhausted"),
            Self::SettingsRootOutsideOsAppData => {
                formatter.write_str("settings root is outside OS application data")
            }
            Self::UnsupportedFormatVersion(version) => {
                write!(formatter, "unsupported settings format version: {version}")
            }
            Self::WriterPoisoned => formatter.write_str("settings writer is unavailable"),
            Self::WriterRegistryPoisoned => {
                formatter.write_str("settings writer registry is unavailable")
            }
        }
    }
}

impl Error for SettingsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidJson(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::DiagnosticNameExhausted
            | Self::InvalidContent { .. }
            | Self::InvalidDirectoryPath
            | Self::InvalidApplicationDirectory
            | Self::LinkedSettingsDirectory
            | Self::MissingSettingsRoot
            | Self::OsAppDataUnavailable
            | Self::RevisionExhausted
            | Self::SettingsRootOutsideOsAppData
            | Self::WriterPoisoned
            | Self::WriterRegistryPoisoned
            | Self::UnsupportedFormatVersion(_) => None,
        }
    }
}
