use super::models::Message;
use super::utils;
use anyhow::Result;
use libsql::Connection;

pub struct MessageRepository {
    connection: Connection,
}

impl MessageRepository {
    pub fn new(connection: Connection) -> Self {
        Self { connection }
    }

    /// Deletes a message from the local store.
    ///
    /// When `chat_id` is `Some`, only that chat is considered (channel deletions).
    /// When `None`, only non-channel chats are considered, because Telegram's
    /// `UpdateDeleteMessages` carries no peer and account message ids are unique
    /// across non-channel chats.
    pub async fn delete_message(&self, id: i64, chat_id: Option<i64>) -> Result<usize> {
        let deleted = match chat_id {
            Some(chat_id) => {
                self.connection
                    .execute(
                        "DELETE FROM messages WHERE id = ? AND chat_id = ?",
                        (id, chat_id),
                    )
                    .await?
            }
            None => {
                self.connection
                    .execute(
                        "DELETE FROM messages WHERE id = ? AND chat_id > ?",
                        (id, -utils::CHANNEL_ID_OFFSET),
                    )
                    .await?
            }
        };

        Ok(deleted as usize)
    }

    pub async fn save_message(&self, msg: Message) -> Result<usize> {
        let inserted = self
            .connection
            .execute(
                "
            INSERT OR IGNORE INTO messages (id, chat_id, sender_id, timestamp, text, processed, topic_id)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            ",
                (
                    msg.id,
                    msg.chat_id,
                    msg.sender_id,
                    msg.timestamp,
                    msg.text,
                    msg.processed as i64,
                    msg.topic_id,
                ),
            )
            .await?;

        Ok(inserted as usize)
    }

    pub async fn update_message_text(&self, id: i64, chat_id: i64, text: String) -> Result<usize> {
        let updated = self
            .connection
            .execute(
                "UPDATE messages SET text = ? WHERE id = ? AND chat_id = ?",
                (text, id, chat_id),
            )
            .await?;

        Ok(updated as usize)
    }

    #[allow(dead_code)]
    pub async fn get_messages(&self, chat_id: i64) -> Result<Vec<Message>> {
        let mut rows = self.connection.query(
            "SELECT id, chat_id, sender_id, timestamp, text, processed, topic_id FROM messages WHERE chat_id = ? ORDER BY timestamp DESC",
            [chat_id],
        ).await?;

        let mut messages = Vec::new();
        while let Some(row) = rows.next().await? {
            messages.push(Message {
                id: row.get(0)?,
                chat_id: row.get(1)?,
                sender_id: row.get(2)?,
                timestamp: row.get(3)?,
                text: row.get(4)?,
                processed: row.get::<i64>(5)? as i32,
                topic_id: row.get(6)?,
            });
        }
        Ok(messages)
    }

    pub async fn cleanup(&self, age_seconds: i64) -> Result<usize> {
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
            - age_seconds;

        let deleted_count = self
            .connection
            .execute("DELETE FROM messages WHERE timestamp < ?", [cutoff])
            .await?;

        // VACUUM is often not supported or problematic in some libsql environments,
        // but it's a separate command if needed.
        // For now, we'll just return the change count.

        Ok(deleted_count as usize)
    }

    /// Returns the highest message id stored for `chat_id`, or `None` if no
    /// message has been persisted for that chat yet.
    ///
    /// Message ids are monotonic per chat, so this acts as a watermark: the
    /// reader backfills history newer than it on startup.
    pub async fn latest_message_id(&self, chat_id: i64) -> Result<Option<i64>> {
        let mut rows = self
            .connection
            .query("SELECT MAX(id) FROM messages WHERE chat_id = ?", [chat_id])
            .await?;

        if let Some(row) = rows.next().await? {
            Ok(row.get::<Option<i64>>(0)?)
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::db::Db;

    async fn setup_db() -> Db {
        Db::new(":memory:").await.unwrap()
    }

    #[tokio::test]
    async fn test_latest_message_id() {
        let db = setup_db().await;
        let repo = MessageRepository::new(db.connection);

        // No rows yet: there is no watermark.
        assert_eq!(repo.latest_message_id(1).await.unwrap(), None);

        repo.save_message(Message {
            id: 10,
            chat_id: 1,
            sender_id: 1,
            timestamp: 100,
            text: "Hello".to_string(),
            processed: 0,
            topic_id: None,
        })
        .await
        .unwrap();

        repo.save_message(Message {
            id: 11,
            chat_id: 1,
            sender_id: 1,
            timestamp: 101,
            text: "World".to_string(),
            processed: 0,
            topic_id: None,
        })
        .await
        .unwrap();

        // The watermark is the newest id for the chat.
        assert_eq!(repo.latest_message_id(1).await.unwrap(), Some(11));
        // A different chat keeps its own (absent) watermark.
        assert_eq!(repo.latest_message_id(2).await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_latest_message_id_is_per_chat() {
        let db = setup_db().await;
        let repo = MessageRepository::new(db.connection);

        repo.save_message(Message {
            id: 5,
            chat_id: 1,
            sender_id: 1,
            timestamp: 100,
            text: "Chat 1".to_string(),
            processed: 0,
            topic_id: None,
        })
        .await
        .unwrap();

        repo.save_message(Message {
            id: 99,
            chat_id: 2,
            sender_id: 1,
            timestamp: 101,
            text: "Chat 2".to_string(),
            processed: 0,
            topic_id: None,
        })
        .await
        .unwrap();

        assert_eq!(repo.latest_message_id(1).await.unwrap(), Some(5));
        assert_eq!(repo.latest_message_id(2).await.unwrap(), Some(99));
    }

    #[tokio::test]
    async fn test_update_message_text() {
        let db = setup_db().await;
        let repo = MessageRepository::new(db.connection);

        let msg = Message {
            id: 10,
            chat_id: 1,
            sender_id: 1,
            timestamp: 100,
            text: "Original".to_string(),
            processed: 0,
            topic_id: None,
        };

        repo.save_message(msg).await.unwrap();

        let updated = repo
            .update_message_text(10, 1, "Edited".to_string())
            .await
            .unwrap();
        assert_eq!(updated, 1);

        let messages = repo.get_messages(1).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "Edited");
    }

    #[tokio::test]
    async fn test_delete_message() {
        let db = setup_db().await;
        let repo = MessageRepository::new(db.connection);

        let channel_id = -utils::CHANNEL_ID_OFFSET - 500;
        let group_id = 1;

        repo.save_message(Message {
            id: 10,
            chat_id: channel_id,
            sender_id: 1,
            timestamp: 100,
            text: "Channel".to_string(),
            processed: 0,
            topic_id: None,
        })
        .await
        .unwrap();

        repo.save_message(Message {
            id: 10,
            chat_id: group_id,
            sender_id: 1,
            timestamp: 101,
            text: "Group".to_string(),
            processed: 0,
            topic_id: None,
        })
        .await
        .unwrap();

        // Account-wide deletions must only touch non-channel chats.
        assert_eq!(repo.delete_message(10, None).await.unwrap(), 1);
        assert!(repo.get_messages(group_id).await.unwrap().is_empty());
        assert_eq!(repo.get_messages(channel_id).await.unwrap().len(), 1);

        // Channel deletions must only touch the given chat.
        assert_eq!(repo.delete_message(10, Some(channel_id)).await.unwrap(), 1);
        assert!(repo.get_messages(channel_id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_cleanup() {
        let db = setup_db().await;
        let repo = MessageRepository::new(db.connection);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        repo.save_message(Message {
            id: 1,
            chat_id: 1,
            sender_id: 1,
            timestamp: now - 1000,
            text: "Old".to_string(),
            processed: 0,
            topic_id: None,
        })
        .await
        .unwrap();

        repo.save_message(Message {
            id: 2,
            chat_id: 1,
            sender_id: 1,
            timestamp: now - 10,
            text: "New".to_string(),
            processed: 0,
            topic_id: None,
        })
        .await
        .unwrap();

        let deleted = repo.cleanup(500).await.unwrap();
        assert_eq!(deleted, 1);

        let latest = repo.latest_message_id(1).await.unwrap();
        assert_eq!(latest, Some(2));
    }
}
