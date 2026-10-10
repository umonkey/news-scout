use super::models::NewsCandidate;
use anyhow::Result;
use libsql::Connection;

pub struct NewsCandidateRepository {
    connection: Connection,
}

impl NewsCandidateRepository {
    pub fn new(connection: Connection) -> Self {
        Self { connection }
    }

    pub async fn enqueue(
        &self,
        chat_name: &str,
        sender_name: &str,
        url: &str,
        text: &str,
    ) -> Result<usize> {
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let inserted = self
            .connection
            .execute(
                "
            INSERT INTO news_candidates (chat_name, sender_name, url, text, created_at)
            VALUES (?, ?, ?, ?, ?)
            ",
                (chat_name, sender_name, url, text, created_at),
            )
            .await?;

        Ok(inserted as usize)
    }

    pub async fn next(&self) -> Result<Option<NewsCandidate>> {
        let mut rows = self
            .connection
            .query(
                "SELECT id, chat_name, sender_name, url, text, created_at FROM news_candidates ORDER BY id ASC LIMIT 1",
                (),
            )
            .await?;

        if let Some(row) = rows.next().await? {
            Ok(Some(NewsCandidate {
                id: row.get(0)?,
                chat_name: row.get(1)?,
                sender_name: row.get(2)?,
                url: row.get(3)?,
                text: row.get(4)?,
                created_at: row.get(5)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn delete(&self, id: i64) -> Result<usize> {
        let deleted = self
            .connection
            .execute("DELETE FROM news_candidates WHERE id = ?", [id])
            .await?;

        Ok(deleted as usize)
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
    async fn test_enqueue_and_next_returns_oldest() {
        let db = setup_db().await;
        let repo = NewsCandidateRepository::new(db.connection);

        repo.enqueue("Chat A", "Sender A", "https://t.me/a/1", "first text")
            .await
            .unwrap();
        repo.enqueue("Chat B", "Sender B", "https://t.me/b/2", "second text")
            .await
            .unwrap();

        let candidate = repo.next().await.unwrap().unwrap();

        assert_eq!(candidate.id, 1);
        assert_eq!(candidate.chat_name, "Chat A");
        assert_eq!(candidate.sender_name, "Sender A");
        assert_eq!(candidate.url, "https://t.me/a/1");
        assert_eq!(candidate.text, "first text");
        assert!(candidate.created_at > 0);
    }

    #[tokio::test]
    async fn test_delete_removes_oldest() {
        let db = setup_db().await;
        let repo = NewsCandidateRepository::new(db.connection);

        repo.enqueue("Chat A", "Sender A", "https://t.me/a/1", "first text")
            .await
            .unwrap();
        repo.enqueue("Chat B", "Sender B", "https://t.me/b/2", "second text")
            .await
            .unwrap();

        let first = repo.next().await.unwrap().unwrap();
        assert_eq!(repo.delete(first.id).await.unwrap(), 1);

        let second = repo.next().await.unwrap().unwrap();
        assert_eq!(second.chat_name, "Chat B");
        assert_eq!(second.text, "second text");
    }

    #[tokio::test]
    async fn test_next_on_empty_table_is_none() {
        let db = setup_db().await;
        let repo = NewsCandidateRepository::new(db.connection);

        assert!(repo.next().await.unwrap().is_none());
    }
}
