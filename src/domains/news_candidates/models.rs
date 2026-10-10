#[derive(Debug, Clone)]
pub struct NewsCandidate {
    pub id: i64,
    pub chat_name: String,
    pub sender_name: String,
    pub url: String,
    pub text: String,
    /// Unix seconds; kept for ordering and observability, not read in code.
    #[allow(dead_code)]
    pub created_at: i64,
}
