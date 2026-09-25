// A DTO field named like a path, carrying it as a string (ADR 0011).
#[derive(Debug, Serialize)]
pub struct LeakyDto {
    pub id: String,
    pub vault_path: String,
}
