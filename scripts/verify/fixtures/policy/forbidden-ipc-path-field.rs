// A DTO handing a path to the webview (ADR 0011).
#[derive(Debug, Serialize)]
pub struct LeakyDto {
    pub id: String,
    pub location: std::path::PathBuf,
}
