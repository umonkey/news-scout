use anyhow::Result;
use libsql::Connection;

pub struct Db {
    pub connection: Connection,
}

impl Db {
    pub async fn new(path: &str) -> Result<Self> {
        let db = libsql::Builder::new_local(path).build().await?;
        let connection = db.connect()?;

        connection
            .execute_batch(
                "
            CREATE TABLE IF NOT EXISTS messages (
                id INTEGER,
                chat_id INTEGER,
                sender_id INTEGER,
                timestamp INTEGER,
                text TEXT,
                processed INTEGER DEFAULT 0,
                topic_id INTEGER,
                PRIMARY KEY (id, chat_id)
            );

            CREATE TABLE IF NOT EXISTS peers (
                id INTEGER PRIMARY KEY,
                name TEXT,
                username TEXT
            );

            CREATE TABLE IF NOT EXISTS reports (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                text TEXT,
                created_at INTEGER,
                sent_at INTEGER
            );

            CREATE TABLE IF NOT EXISTS news_candidates (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat_name TEXT NOT NULL,
                sender_name TEXT NOT NULL,
                url TEXT NOT NULL,
                text TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

            PRAGMA journal_mode = WAL;
            PRAGMA busy_timeout = 5000;
        ",
            )
            .await?;

        // Lazy migration for existing databases
        let _ = connection
            .execute(
                "ALTER TABLE messages ADD COLUMN processed INTEGER DEFAULT 0;",
                (),
            )
            .await;
        let _ = connection
            .execute("ALTER TABLE peers ADD COLUMN username TEXT;", ())
            .await;
        let _ = connection
            .execute("ALTER TABLE messages ADD COLUMN topic_id INTEGER;", ())
            .await;

        Ok(Self { connection })
    }
}
