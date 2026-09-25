// A path type smuggled through an alias and a tuple struct (ADR 0011).
type Folder = std::path::PathBuf;
#[derive(Serialize)]
pub struct Wrapped(pub Folder);
