//! Fail-closed SQLite connection and transaction kernel for Product Ledger.
//!
//! This boundary deliberately exposes neither SQL nor a raw SQLite connection.

use std::{
    fmt,
    fs::File,
    io::{ErrorKind, Read},
    path::{Component, Path, Prefix},
    time::Duration,
};

use rusqlite::{
    backup::Backup, ffi, Connection, Error as SqliteError, ErrorCode, OpenFlags, OptionalExtension,
    TransactionBehavior,
};
use sha2::{Digest, Sha256};

mod inspection;
mod migrations;
mod schema;
mod upgrade;

mod action_repository;
mod composition_repository;
mod decision_repository;
mod delivery_repository;
mod evidence_repository;
mod issue_repository;
mod managed_projection_rebuild_repository;
mod portfolio_repository;
mod projection_repository;
mod record_entry_repository;
mod relationship_h2b_repository;
mod relationship_repository;
mod reservation_repository;
mod risk_repository;

pub use action_repository::ActionPersistenceLoadError;
pub use decision_repository::DecisionPersistenceLoadError;
pub use delivery_repository::DeliveryMutationOutcome;
pub use delivery_repository::DeliveryPersistenceLoadError;
pub use evidence_repository::{EvidenceMatch, EvidenceVaultEntry};
pub use inspection::{
    inspect_ledger, inspect_standalone_snapshot, snapshot_ledger_file, LedgerCompatibility,
    LedgerInspection, FIRST_UPGRADEABLE_SCHEMA_VERSION,
};
pub use issue_repository::IssuePersistenceLoadError;
pub use record_entry_repository::{
    ActionRequestDraftFields, DecisionRequestDraftFields, InitiativeEntryRecord, IssueFields,
    KpiDefinitionEntryRecord, KpiObservationEntryRecord, MilestoneEntryRecord, ProjectEntryRecord,
    SimpleEntryRecord, StakeholderEntryRecord, CREATE_ACTION_REQUEST_DRAFT,
    CREATE_DECISION_REQUEST_DRAFT, CREATE_INITIATIVE, CREATE_ISSUE, CREATE_KPI_DEFINITION,
    CREATE_KPI_OBSERVATION, CREATE_MILESTONE, CREATE_PORTFOLIO, CREATE_PRODUCT, CREATE_PROJECT,
    CREATE_ROADMAP, CREATE_STAKEHOLDER, LINK_INITIATIVE_PROJECT, LINK_PORTFOLIO_PRODUCT,
    LINK_PRODUCT_KPI, LINK_PRODUCT_ROADMAP, LINK_PROJECT_PRODUCT, LINK_STAKEHOLDER_SUBJECT,
};
pub use reservation_repository::{
    ReservableId, ReservationError, ReservationRequest, ReservedEntityKind, ReservedId,
};
pub use risk_repository::RiskPersistenceLoadError;
pub use upgrade::{
    upgrade_in_place, LedgerUpgradeError, UpgradeOutcome, UpgradeSource, VerifiedPreUpgradeBackup,
};

pub const APPLICATION_ID: u32 = 0x504D_4301;
pub const CURRENT_SCHEMA_VERSION: u32 = 48;
const BUSY_TIMEOUT: Duration = Duration::from_millis(5_000);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalMode {
    Wal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SynchronousMode {
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectionPolicy {
    pub foreign_keys: bool,
    pub journal_mode: JournalMode,
    pub synchronous: SynchronousMode,
    pub busy_timeout: Duration,
    pub trusted_schema: bool,
    pub extension_loading_denied: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerOpenError {
    UnclaimedDatabase,
    WrongApplication,
    FutureSchema { found: u32 },
    UnsupportedSchema { found: u32 },
    InvalidMetadata,
    CorruptDatabase,
    PolicyViolation,
    Busy,
    StorageUnavailable,
}

impl fmt::Display for LedgerOpenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnclaimedDatabase => "the database is not an unclaimed empty Product Ledger",
            Self::WrongApplication => "the database belongs to another application",
            Self::FutureSchema { .. } => "the Product Ledger schema is newer than this application",
            Self::UnsupportedSchema { .. } => "the Product Ledger schema is unsupported",
            Self::InvalidMetadata => "the Product Ledger metadata is invalid",
            Self::CorruptDatabase => "the Product Ledger cannot be read safely",
            Self::PolicyViolation => "the required Product Ledger connection policy is unavailable",
            Self::Busy => "the Product Ledger is busy",
            Self::StorageUnavailable => "the Product Ledger storage is unavailable",
        })
    }
}

impl std::error::Error for LedgerOpenError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionConflict;

#[derive(Debug, Eq, PartialEq)]
pub enum LedgerTransactionError<E> {
    Operation(E),
    RevisionConflict,
    IncompatibleLedger,
    Busy,
    CommitFailed,
}

/// The first line of every authority inventory; a later format changes it.
pub const AUTHORITY_INVENTORY_HEADER: &str = "pmc-authority-inventory/v1\n";

/// See [`SqliteProductLedger::authority_inventory`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorityInventory {
    pub bytes: Vec<u8>,
    /// SHA-256 of `bytes`, lower-case hex.
    pub sha256: String,
    pub record_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerSnapshotManifest {
    schema_version: u32,
    ledger_revision: u64,
    sha256: String,
}

impl LedgerSnapshotManifest {
    /// A manifest as an archive states it (a restore reads it from the
    /// archive's own manifest). It proves nothing by itself: every use checks
    /// the snapshot against it with `inspect_standalone_snapshot`.
    #[must_use]
    pub const fn claimed(schema_version: u32, ledger_revision: u64, sha256: String) -> Self {
        Self {
            schema_version,
            ledger_revision,
            sha256,
        }
    }

    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub const fn ledger_revision(&self) -> u64 {
        self.ledger_revision
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerSnapshotError {
    DestinationAlreadyExists,
    DestinationUnavailable,
    SnapshotFailed,
    VerificationFailed,
}

/// Narrow S2 backup kernel used only to make and verify a named Ledger snapshot.
/// It intentionally has no restore, overwrite, retention, or profile authority.
pub trait OperationalBackupPort {
    fn create_verified_snapshot(
        &self,
        destination: &Path,
    ) -> Result<LedgerSnapshotManifest, LedgerSnapshotError>;

    fn verify_snapshot(
        &self,
        snapshot: &Path,
        manifest: &LedgerSnapshotManifest,
    ) -> Result<(), LedgerSnapshotError>;
}

pub struct SqliteProductLedger {
    connection: Connection,
    schema_version: u32,
}

pub struct LedgerWriteTransaction<'transaction> {
    transaction: rusqlite::Transaction<'transaction>,
}

impl SqliteProductLedger {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LedgerOpenError> {
        let path = path.as_ref();
        let state = inspect_path(path)?;
        if state == Inspection::Empty {
            initialize_empty(path)?;
        }

        let verified = inspect_path(path)?;
        let schema_version = match verified {
            Inspection::Ledger { schema_version } => schema_version,
            Inspection::Empty => return Err(LedgerOpenError::InvalidMetadata),
        };
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(classify_open_error)?;
        if inspect_connection(&connection)? != verified {
            return Err(LedgerOpenError::InvalidMetadata);
        }
        apply_and_verify_policy(&connection)?;
        Ok(Self {
            connection,
            schema_version,
        })
    }

    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn revision(&self) -> Result<u64, LedgerOpenError> {
        read_revision(&self.connection).map_err(|_| LedgerOpenError::InvalidMetadata)
    }

    /// When any authority record last changed (the latest `updated_at` in
    /// the aggregate registry), or `None` when the Ledger holds none. What
    /// the restore preview shows as the current Ledger's last change.
    pub fn last_change_at_millis(&self) -> Result<Option<i64>, LedgerOpenError> {
        self.connection
            .query_row(
                "SELECT MAX(updated_at) FROM aggregate_registry",
                [],
                |row| row.get(0),
            )
            .map_err(|_| LedgerOpenError::InvalidMetadata)
    }

    pub fn connection_policy(&self) -> Result<ConnectionPolicy, LedgerOpenError> {
        observe_policy(&self.connection)
    }

    /// Every aggregate's type, id and version, sorted, in one canonical
    /// text: what an Operational Backup carries so a restore can compare the
    /// exact authority versions it expects record by record (S7 plan §7),
    /// rather than one `ledger_revision` counter that says something
    /// changed but not what.
    pub fn authority_inventory(&self) -> Result<AuthorityInventory, LedgerOpenError> {
        authority_inventory_of(&self.connection)
    }

    /// Whether the Ledger holds nothing but its own metadata: no aggregate,
    /// no Prepared Intent, no audit, receipt or replay row — every table
    /// except the schema's own metadata (`ledger_metadata`,
    /// `schema_migrations`, filled when the Ledger is created) is empty. The
    /// named bootstrap of S7 §6 may only ever apply to such a Ledger.
    pub fn is_pristine_authority(&self) -> Result<bool, LedgerOpenError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT name FROM sqlite_schema WHERE type = 'table' \
                 AND name NOT LIKE 'sqlite_%' \
                 AND name NOT IN ('ledger_metadata', 'schema_migrations')",
            )
            .map_err(|_| LedgerOpenError::CorruptDatabase)?;
        let tables = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| LedgerOpenError::CorruptDatabase)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| LedgerOpenError::CorruptDatabase)?;
        for table in tables {
            let quoted = table.replace('"', "\"\"");
            let occupied: bool = self
                .connection
                .query_row(
                    &format!("SELECT EXISTS(SELECT 1 FROM \"{quoted}\")"),
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| LedgerOpenError::CorruptDatabase)?;
            if occupied {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn create_verified_snapshot(
        &self,
        destination: impl AsRef<Path>,
    ) -> Result<LedgerSnapshotManifest, LedgerSnapshotError> {
        OperationalBackupPort::create_verified_snapshot(self, destination.as_ref())
    }

    pub fn verify_snapshot(
        &self,
        snapshot: impl AsRef<Path>,
        manifest: &LedgerSnapshotManifest,
    ) -> Result<(), LedgerSnapshotError> {
        OperationalBackupPort::verify_snapshot(self, snapshot.as_ref(), manifest)
    }

    pub fn with_immediate_transaction<T, E, F>(
        &mut self,
        operation: F,
    ) -> Result<T, LedgerTransactionError<E>>
    where
        F: FnOnce(&mut LedgerWriteTransaction<'_>) -> Result<T, E>,
    {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(classify_transaction_start)?;
        match inspect_connection(&transaction) {
            Ok(Inspection::Ledger { schema_version }) if schema_version == self.schema_version => {}
            _ => return Err(LedgerTransactionError::IncompatibleLedger),
        }
        let mut transaction = LedgerWriteTransaction { transaction };
        let result = operation(&mut transaction).map_err(LedgerTransactionError::Operation)?;
        transaction
            .transaction
            .commit()
            .map_err(classify_transaction_commit)?;
        Ok(result)
    }
}

impl OperationalBackupPort for SqliteProductLedger {
    fn create_verified_snapshot(
        &self,
        destination: &Path,
    ) -> Result<LedgerSnapshotManifest, LedgerSnapshotError> {
        if destination
            .try_exists()
            .map_err(|_| LedgerSnapshotError::DestinationUnavailable)?
        {
            return Err(LedgerSnapshotError::DestinationAlreadyExists);
        }
        let revision = self
            .revision()
            .map_err(|_| LedgerSnapshotError::VerificationFailed)?;
        File::options()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|error| {
                if error.kind() == ErrorKind::AlreadyExists {
                    LedgerSnapshotError::DestinationAlreadyExists
                } else {
                    LedgerSnapshotError::DestinationUnavailable
                }
            })?;
        let mut snapshot = Connection::open_with_flags(
            destination,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| LedgerSnapshotError::DestinationUnavailable)?;
        let backup = Backup::new(&self.connection, &mut snapshot)
            .map_err(|_| LedgerSnapshotError::SnapshotFailed)?;
        backup
            .run_to_completion(128, Duration::from_millis(1), None)
            .map_err(|_| LedgerSnapshotError::SnapshotFailed)?;
        drop(backup);
        drop(snapshot);

        // Verified read-only: reopening it through `open` would write WAL
        // files beside it and would refuse it once this binary is newer.
        let manifest = LedgerSnapshotManifest {
            schema_version: self.schema_version,
            ledger_revision: revision,
            sha256: snapshot_sha256(destination)?,
        };
        inspection::inspect_standalone_snapshot(destination, &manifest)?;
        Ok(manifest)
    }

    fn verify_snapshot(
        &self,
        snapshot: &Path,
        manifest: &LedgerSnapshotManifest,
    ) -> Result<(), LedgerSnapshotError> {
        inspection::inspect_standalone_snapshot(snapshot, manifest).map(|_| ())
    }
}

fn snapshot_sha256(path: &Path) -> Result<String, LedgerSnapshotError> {
    let mut file = File::open(path).map_err(|_| LedgerSnapshotError::VerificationFailed)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16_384];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| LedgerSnapshotError::VerificationFailed)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

impl LedgerWriteTransaction<'_> {
    pub fn advance_revision(&mut self, expected: u64) -> Result<u64, RevisionConflict> {
        let next = expected.checked_add(1).ok_or(RevisionConflict)?;
        let expected = i64::try_from(expected).map_err(|_| RevisionConflict)?;
        let next_sql = i64::try_from(next).map_err(|_| RevisionConflict)?;
        let changed = self
            .transaction
            .execute(
                "UPDATE ledger_metadata SET ledger_revision = ?1 WHERE singleton = 1 AND ledger_revision = ?2",
                (next_sql, expected),
            )
            .map_err(|_| RevisionConflict)?;
        if changed == 1 {
            Ok(next)
        } else {
            Err(RevisionConflict)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Inspection {
    Empty,
    Ledger { schema_version: u32 },
}

fn inspect_path(path: &Path) -> Result<Inspection, LedgerOpenError> {
    if !path.exists() {
        return Ok(Inspection::Empty);
    }
    let length = path
        .metadata()
        .map_err(|_| LedgerOpenError::StorageUnavailable)?
        .len();
    if length == 0 {
        return Ok(Inspection::Empty);
    }
    let uri = immutable_read_uri(path)?;
    let connection = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(classify_open_error)?;
    inspect_connection(&connection)
}

/// Serialize `path` as the read-only, `immutable=1` inspection URI.
///
/// Windows canonical paths (everything `fs::canonicalize` returns, and so
/// everything `ProtectedSettingsRoot`/`WorkspaceIdentity`/`VaultRoot` hand
/// out) carry the verbatim `\\?\` prefix. That prefix is preserved here, not
/// stripped: it requests verbatim Windows name handling -- long paths past
/// `MAX_PATH`, and no normalization of trailing spaces/dots -- so removing it
/// would silently change which file is addressed. SQLite's bundled Windows
/// VFS already understands a URI-decoded `/\\?\...` and drops only the
/// URI-introduced leading slash.
///
/// Two structural rules keep this safe:
///
/// 1. **Separators are never rewritten.** The previous implementation
///    replaced `\` with `/`, which turned `\\?\C:\...` into `//?/C:/...` and
///    made SQLite parse `%3F` as a URI *authority* (`invalid uri authority`),
///    surfacing as an opaque `StorageUnavailable`.
/// 2. **`/` is percent-encoded and the URI always starts `file:/`.** The byte
///    after that slash therefore can never itself be `/`, so `file://` -- the
///    only form that introduces an authority -- is impossible to construct
///    regardless of the input path.
///
/// The allowlist stays deliberately narrow: `?` and `#` delimit query and
/// fragment, `%` would let a caller-controlled path inject escapes, and `&`
/// and `=` would let it inject query options. The `?mode=ro&immutable=1`
/// suffix is a fixed literal and never caller-influenced.
///
/// Fails closed rather than guessing for two inputs: a non-UTF-8 path (which
/// a lossy conversion would turn into a *different* pathname), and a UNC or
/// verbatim-UNC path. Network-hosted Ledger locations are not an accidental
/// URI-syntax failure -- they are deliberately out of scope until the
/// production authority-path gate accepts supported-location
/// controls, so they are rejected as a policy violation here.
fn immutable_read_uri(path: &Path) -> Result<String, LedgerOpenError> {
    if matches!(
        path.components().next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..))
    ) {
        return Err(LedgerOpenError::PolicyViolation);
    }
    let path = path.to_str().ok_or(LedgerOpenError::PolicyViolation)?;
    // `file:/` supplies the leading slash, so any the path already carries
    // (an ordinary absolute POSIX path) would otherwise be doubled.
    let path = path.trim_start_matches('/');
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-' | b'_' | b'.') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(format!("file:/{encoded}?mode=ro&immutable=1"))
}

fn inspect_connection(connection: &Connection) -> Result<Inspection, LedgerOpenError> {
    configure_inspection_safety(connection)?;
    let application_id = pragma_i64(connection, "application_id")?;
    let user_version = pragma_i64(connection, "user_version")?;
    let page_count = pragma_i64(connection, "page_count")?;
    let freelist_count = pragma_i64(connection, "freelist_count")?;
    let schema_objects = schema::schema_objects(connection)?;
    if application_id == 0
        && user_version == 0
        && page_count <= 1
        && freelist_count == 0
        && schema_objects.is_empty()
    {
        return Ok(Inspection::Empty);
    }
    if application_id == 0 {
        return Err(LedgerOpenError::UnclaimedDatabase);
    }
    if application_id != i64::from(APPLICATION_ID) {
        return Err(LedgerOpenError::WrongApplication);
    }
    let schema_version =
        u32::try_from(user_version).map_err(|_| LedgerOpenError::InvalidMetadata)?;
    if schema_version > CURRENT_SCHEMA_VERSION {
        return Err(LedgerOpenError::FutureSchema {
            found: schema_version,
        });
    }
    if schema_version != CURRENT_SCHEMA_VERSION {
        return Err(LedgerOpenError::UnsupportedSchema {
            found: schema_version,
        });
    }
    schema::validate_canonical_schema(connection)?;
    migrations::validate_registry(connection)?;
    let metadata: Option<(i64, i64, i64)> = connection
        .query_row(
            "SELECT singleton, schema_version, ledger_revision FROM ledger_metadata",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|_| LedgerOpenError::InvalidMetadata)?;
    match metadata {
        Some((1, metadata_version, revision))
            if metadata_version == user_version && revision >= 0 => {}
        _ => return Err(LedgerOpenError::InvalidMetadata),
    }
    let metadata_rows: i64 = connection
        .query_row("SELECT count(*) FROM ledger_metadata", [], |row| row.get(0))
        .map_err(|_| LedgerOpenError::InvalidMetadata)?;
    if metadata_rows != 1 {
        return Err(LedgerOpenError::InvalidMetadata);
    }
    Ok(Inspection::Ledger { schema_version })
}

fn initialize_empty(path: &Path) -> Result<(), LedgerOpenError> {
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(classify_open_error)?;
    if !is_pristine_before_bootstrap_lock(&connection)? {
        return Err(LedgerOpenError::UnclaimedDatabase);
    }
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(classify_open_error)?;
    if inspect_connection(&transaction)? != Inspection::Empty {
        return Err(LedgerOpenError::UnclaimedDatabase);
    }
    migrations::bootstrap(&transaction)?;
    transaction.commit().map_err(classify_open_error)
}

fn is_pristine_before_bootstrap_lock(connection: &Connection) -> Result<bool, LedgerOpenError> {
    configure_inspection_safety(connection)?;
    Ok(pragma_i64(connection, "application_id")? == 0
        && pragma_i64(connection, "user_version")? == 0
        && pragma_i64(connection, "page_count")? == 0
        && pragma_i64(connection, "freelist_count")? == 0
        && schema::schema_objects(connection)?.is_empty())
}

fn configure_inspection_safety(connection: &Connection) -> Result<(), LedgerOpenError> {
    connection
        .execute_batch("PRAGMA trusted_schema = OFF;")
        .map_err(classify_inspection_error)?;
    if pragma_i64(connection, "trusted_schema")? != 0 {
        return Err(LedgerOpenError::PolicyViolation);
    }
    Ok(())
}

fn apply_and_verify_policy(connection: &Connection) -> Result<(), LedgerOpenError> {
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(classify_open_error)?;
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA synchronous = FULL;
             PRAGMA trusted_schema = OFF;",
        )
        .map_err(classify_open_error)?;
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(classify_open_error)?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        return Err(LedgerOpenError::PolicyViolation);
    }
    if !extension_denial_probe(connection) {
        return Err(LedgerOpenError::PolicyViolation);
    }
    observe_policy(connection).map(|_| ())
}

fn observe_policy(connection: &Connection) -> Result<ConnectionPolicy, LedgerOpenError> {
    let foreign_keys = pragma_i64(connection, "foreign_keys")? == 1;
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(classify_open_error)?;
    let synchronous = pragma_i64(connection, "synchronous")?;
    let busy_timeout = pragma_i64(connection, "busy_timeout")?;
    let trusted_schema = pragma_i64(connection, "trusted_schema")? == 1;
    let policy = ConnectionPolicy {
        foreign_keys,
        journal_mode: JournalMode::Wal,
        synchronous: SynchronousMode::Full,
        busy_timeout: Duration::from_millis(
            u64::try_from(busy_timeout).map_err(|_| LedgerOpenError::PolicyViolation)?,
        ),
        trusted_schema,
        extension_loading_denied: extension_denial_probe(connection),
    };
    if !foreign_keys
        || !journal_mode.eq_ignore_ascii_case("wal")
        || synchronous != 2
        || policy.busy_timeout != BUSY_TIMEOUT
        || trusted_schema
        || !policy.extension_loading_denied
    {
        return Err(LedgerOpenError::PolicyViolation);
    }
    Ok(policy)
}

fn extension_denial_probe(connection: &Connection) -> bool {
    match connection.query_row::<i64, _, _>("SELECT load_extension('')", [], |row| row.get(0)) {
        Err(SqliteError::SqliteFailure(_, Some(message))) => message == "not authorized",
        Err(SqliteError::SqliteFailure(error, None)) => error.extended_code == ffi::SQLITE_AUTH,
        _ => false,
    }
}

/// See [`SqliteProductLedger::authority_inventory`]; usable on any
/// connection, including a read-only inspection of an older Ledger.
pub(crate) fn authority_inventory_of(
    connection: &Connection,
) -> Result<AuthorityInventory, LedgerOpenError> {
    let mut statement = connection
        .prepare(
            "SELECT aggregate_type, id, version FROM aggregate_registry \
             ORDER BY aggregate_type, id",
        )
        .map_err(|_| LedgerOpenError::CorruptDatabase)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|_| LedgerOpenError::CorruptDatabase)?;
    let mut bytes = AUTHORITY_INVENTORY_HEADER.as_bytes().to_vec();
    let mut record_count = 0_u64;
    for row in rows {
        let (aggregate_type, id, version) = row.map_err(|_| LedgerOpenError::CorruptDatabase)?;
        // Ids and types are constrained tokens; a tab or newline in one
        // would make the text ambiguous, so it is refused.
        if [&aggregate_type, &id]
            .iter()
            .any(|field| field.contains(['\t', '\n', '\r']))
        {
            return Err(LedgerOpenError::CorruptDatabase);
        }
        bytes.extend_from_slice(format!("{aggregate_type}\t{id}\t{version}\n").as_bytes());
        record_count += 1;
    }
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    Ok(AuthorityInventory {
        bytes,
        sha256,
        record_count,
    })
}

fn read_revision(connection: &Connection) -> Result<u64, SqliteError> {
    let value: i64 = connection.query_row(
        "SELECT ledger_revision FROM ledger_metadata WHERE singleton = 1",
        [],
        |row| row.get(0),
    )?;
    u64::try_from(value).map_err(|_| SqliteError::IntegralValueOutOfRange(0, value))
}

fn pragma_i64(connection: &Connection, name: &str) -> Result<i64, LedgerOpenError> {
    connection
        .query_row(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .map_err(classify_inspection_error)
}

fn classify_inspection_error(error: SqliteError) -> LedgerOpenError {
    match error {
        SqliteError::SqliteFailure(_, _) | SqliteError::InvalidQuery => {
            LedgerOpenError::CorruptDatabase
        }
        _ => LedgerOpenError::StorageUnavailable,
    }
}

fn classify_open_error(error: SqliteError) -> LedgerOpenError {
    match error {
        SqliteError::SqliteFailure(ref details, _) if details.code == ErrorCode::DatabaseBusy => {
            LedgerOpenError::Busy
        }
        SqliteError::SqliteFailure(ref details, _) => {
            record_storage_unavailable_diagnostic(details);
            LedgerOpenError::StorageUnavailable
        }
        _ => LedgerOpenError::StorageUnavailable,
    }
}

/// Local-only diagnostic for an open-time SQLite failure that
/// `classify_open_error` maps to the generic, safe `StorageUnavailable`
/// code. Deliberately omits the file path and SQLite's free-form message --
/// either can contain the on-disk location -- and records only the
/// coarse-grained primary `ErrorCode` classification plus the numeric
/// extended code, which is enough to tell causes apart locally (a bad
/// generated URI, a locked file, a missing directory, ...) without any of
/// that detail crossing the `SafeErrorDto`/IPC boundary, since only the
/// caller's stderr ever sees this line.
fn record_storage_unavailable_diagnostic(details: &ffi::Error) {
    eprintln!(
        "pmc_ledger sqlite_open_storage_unavailable primary_code={:?} extended_code={}",
        details.code, details.extended_code
    );
}

fn classify_transaction_start<E>(error: SqliteError) -> LedgerTransactionError<E> {
    match error {
        SqliteError::SqliteFailure(ref details, _) if details.code == ErrorCode::DatabaseBusy => {
            LedgerTransactionError::Busy
        }
        _ => LedgerTransactionError::CommitFailed,
    }
}

fn classify_transaction_commit<E>(error: SqliteError) -> LedgerTransactionError<E> {
    match error {
        SqliteError::SqliteFailure(ref details, _) if details.code == ErrorCode::DatabaseBusy => {
            LedgerTransactionError::Busy
        }
        _ => LedgerTransactionError::CommitFailed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The regression this whole builder exists to prevent: a canonical
    /// Windows path (what `fs::canonicalize`, and therefore every protected
    /// root in `pmc-platform`, actually produces) must not have its verbatim
    /// prefix turned into a URI authority.
    #[test]
    fn a_verbatim_windows_path_keeps_its_prefix_and_introduces_no_authority() {
        let uri = immutable_read_uri(Path::new(r"\\?\C:\dir\product-ledger.sqlite3"))
            .unwrap_or_else(|_| panic!("a verbatim drive path must serialize"));
        assert_eq!(
            uri,
            "file:/%5C%5C%3F%5CC:%5Cdir%5Cproduct-ledger.sqlite3?mode=ro&immutable=1"
        );
        assert!(!uri.starts_with("file://"));
    }

    #[test]
    fn an_ordinary_windows_path_encodes_its_separators() {
        let uri = immutable_read_uri(Path::new(r"C:\dir\product-ledger.sqlite3"))
            .unwrap_or_else(|_| panic!("an ordinary drive path must serialize"));
        assert_eq!(
            uri,
            "file:/C:%5Cdir%5Cproduct-ledger.sqlite3?mode=ro&immutable=1"
        );
    }

    #[test]
    fn an_absolute_posix_path_is_not_double_slashed() {
        let uri = immutable_read_uri(Path::new("/var/lib/product-ledger.sqlite3"))
            .unwrap_or_else(|_| panic!("an absolute POSIX path must serialize"));
        assert_eq!(
            uri,
            "file:/var%2Flib%2Fproduct-ledger.sqlite3?mode=ro&immutable=1"
        );
        assert!(!uri.starts_with("file://"));
    }

    /// Network-hosted Ledgers stay out of scope until the production
    /// authority-path gate accepts supported-location controls, so they must
    /// fail as a deliberate policy decision rather than as an accidental URI
    /// syntax error.
    /// Windows-only by nature, not by convenience: `std::path::Prefix` is
    /// parsed on Windows alone, so on Linux these strings are ordinary file
    /// names containing backslashes and there is no UNC path to refuse. This
    /// test failed on every CI run from 2026-09-03 for exactly that reason.
    #[cfg(windows)]
    #[test]
    fn unc_and_verbatim_unc_paths_are_refused_as_a_policy_violation() {
        for path in [
            r"\\server\share\product-ledger.sqlite3",
            r"\\?\UNC\server\share\product-ledger.sqlite3",
        ] {
            assert_eq!(
                immutable_read_uri(Path::new(path)),
                Err(LedgerOpenError::PolicyViolation),
                "{path} must be refused"
            );
        }
    }

    /// Every byte that could otherwise terminate the path component or inject
    /// a query option has to survive as encoded path text, leaving the fixed
    /// `?mode=ro&immutable=1` suffix as the only query in the URI.
    #[test]
    fn reserved_uri_bytes_in_a_path_cannot_reach_the_query() {
        let uri = immutable_read_uri(Path::new(r"C:\d?x#y%z&mode=rw\ledger .sqlite3"))
            .unwrap_or_else(|_| panic!("a path with reserved bytes must serialize"));
        for encoded in ["%3F", "%23", "%25", "%26", "%3D", "%20"] {
            assert!(uri.contains(encoded), "{encoded} missing from {uri}");
        }
        assert_eq!(uri.matches('?').count(), 1);
        assert!(uri.ends_with("?mode=ro&immutable=1"));
    }

    #[test]
    fn a_non_ascii_path_is_percent_encoded_rather_than_rewritten() {
        let uri = immutable_read_uri(Path::new(r"C:\研發\ledger.sqlite3"))
            .unwrap_or_else(|_| panic!("a non-ASCII path must serialize"));
        assert!(uri.starts_with("file:/C:%5C"));
        assert!(!uri.contains('研'));
        assert!(uri.ends_with("%5Cledger.sqlite3?mode=ro&immutable=1"));
    }

    #[test]
    fn every_extended_busy_code_is_classified_as_busy() {
        for code in [
            ffi::SQLITE_BUSY_RECOVERY,
            ffi::SQLITE_BUSY_SNAPSHOT,
            ffi::SQLITE_BUSY_TIMEOUT,
        ] {
            let error = SqliteError::SqliteFailure(ffi::Error::new(code), None);
            assert_eq!(
                classify_transaction_start::<()>(error),
                LedgerTransactionError::Busy
            );
            let error = SqliteError::SqliteFailure(ffi::Error::new(code), None);
            assert_eq!(classify_open_error(error), LedgerOpenError::Busy);
        }
    }

    /// A non-busy SQLite open failure still classifies as the generic, safe
    /// `StorageUnavailable` -- the sanitized diagnostic recorded alongside it
    /// is local-only and must never change this public classification.
    #[test]
    fn a_non_busy_sqlite_failure_still_classifies_as_storage_unavailable() {
        let error = SqliteError::SqliteFailure(ffi::Error::new(ffi::SQLITE_CANTOPEN), None);
        assert_eq!(
            classify_open_error(error),
            LedgerOpenError::StorageUnavailable
        );
    }
}
