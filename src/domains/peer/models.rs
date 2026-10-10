#[derive(Debug, Clone)]
pub struct Peer {
    pub id: i64,
    pub name: String,
    pub username: Option<String>,
}
