use super::models::Peer;
use anyhow::Result;
use libsql::Connection;

pub struct PeerRepository {
    connection: Connection,
}

impl PeerRepository {
    pub fn new(connection: Connection) -> Self {
        Self { connection }
    }

    pub async fn save_peer(&self, peer: Peer) -> Result<()> {
        let res = self
            .connection
            .execute(
                "INSERT INTO peers (id, name, username) VALUES (?, ?, ?)",
                (peer.id, peer.name.clone(), peer.username.clone()),
            )
            .await;

        match res {
            Ok(_) => {
                log::info!("New peer found: {} ({})", peer.name, peer.id);
            }
            Err(e) if e.to_string().contains("UNIQUE constraint failed") => {
                self.connection
                    .execute(
                        "UPDATE peers SET name = ?, username = ? WHERE id = ?",
                        (peer.name, peer.username, peer.id),
                    )
                    .await?;
            }
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn get_peer(&self, id: i64) -> Result<Option<Peer>> {
        let mut rows = self
            .connection
            .query("SELECT id, name, username FROM peers WHERE id = ?", [id])
            .await?;

        if let Some(row) = rows.next().await? {
            Ok(Some(Peer {
                id: row.get(0)?,
                name: row.get(1)?,
                username: row.get(2)?,
            }))
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
    async fn test_save_and_get_peer() {
        let db = setup_db().await;
        let repo = PeerRepository::new(db.connection);

        let peer = Peer {
            id: 123,
            name: "Test User".to_string(),
            username: Some("testuser".to_string()),
        };

        repo.save_peer(peer.clone()).await.unwrap();

        let saved = repo.get_peer(123).await.unwrap().unwrap();
        assert_eq!(saved.name, "Test User");
        assert_eq!(saved.username, Some("testuser".to_string()));
    }

    #[tokio::test]
    async fn test_upsert_peer() {
        let db = setup_db().await;
        let repo = PeerRepository::new(db.connection);

        let peer1 = Peer {
            id: 123,
            name: "Original Name".to_string(),
            username: None,
        };
        repo.save_peer(peer1).await.unwrap();

        let peer2 = Peer {
            id: 123,
            name: "Updated Name".to_string(),
            username: Some("newuser".to_string()),
        };
        repo.save_peer(peer2).await.unwrap();

        let saved = repo.get_peer(123).await.unwrap().unwrap();
        assert_eq!(saved.name, "Updated Name");
        assert_eq!(saved.username, Some("newuser".to_string()));
    }
}
