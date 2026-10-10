#[derive(Debug, Clone)]
pub struct Message {
    pub id: i64,
    pub chat_id: i64,
    pub sender_id: i64,
    pub timestamp: i64,
    pub text: String,
    pub processed: i32,
    pub topic_id: Option<i64>,
}
