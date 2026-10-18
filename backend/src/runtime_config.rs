//! SQLite-backed runtime data limits.
//!
//! The password and instance registry intentionally remain in `admin.json`.
//! This database only stores non-secret runtime policy so it can be changed
//! without editing environment variables or restarting the process.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

const MAX_LIMIT: usize = 10_000_000;
const DB_FILE: &str = "runtime.sqlite3";

fn default_message_reset_secs() -> usize {
    86_400
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeConfig {
    /// 0 means unlimited.
    pub max_messages: usize,
    /// 0 means unlimited.
    pub max_routes: usize,
    /// 0 means unlimited.
    pub max_peers: usize,
    /// 0 disables periodic message-counter reset.
    #[serde(default = "default_message_reset_secs")]
    pub message_reset_secs: usize,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_messages: 500,
            max_routes: 0,
            max_peers: 0,
            message_reset_secs: 86_400,
        }
    }
}

impl RuntimeConfig {
    fn validate(self) -> anyhow::Result<Self> {
        for (name, value) in [
            ("maxMessages", self.max_messages),
            ("maxRoutes", self.max_routes),
            ("maxPeers", self.max_peers),
        ] {
            if value > MAX_LIMIT {
                bail!("{name} must be 0 (unlimited) or at most {MAX_LIMIT}");
            }
        }
        if self.message_reset_secs > 31_536_000 {
            bail!("messageResetSecs must be 0 (disabled) or at most 31536000 (one year)");
        }
        Ok(self)
    }
}

pub struct RuntimeConfigStore {
    conn: Mutex<Connection>,
    config: Mutex<RuntimeConfig>,
}

impl RuntimeConfigStore {
    pub fn load(data_dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        let data_dir = data_dir.as_ref();
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("failed to create data dir {}", data_dir.display()))?;
        let path = data_dir.join(DB_FILE);
        let conn = Connection::open(&path)
            .with_context(|| format!("failed to open SQLite database {}", path.display()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS runtime_config (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                max_messages INTEGER NOT NULL,
                max_routes INTEGER NOT NULL,
                max_peers INTEGER NOT NULL,
                message_reset_secs INTEGER NOT NULL DEFAULT 86400
            );",
        )
        .with_context(|| format!("failed to initialize SQLite database {}", path.display()))?;

        // Migrate databases created before message_reset_secs existed. This
        // must run before the INSERT statement references the new column.
        // The duplicate-column error on fresh databases is intentionally
        // ignored.
        let _ = conn.execute(
            "ALTER TABLE runtime_config
             ADD COLUMN message_reset_secs INTEGER NOT NULL DEFAULT 86400",
            [],
        );
        conn.execute(
            "INSERT OR IGNORE INTO runtime_config
                (id, max_messages, max_routes, max_peers, message_reset_secs)
                VALUES (1, 500, 0, 0, 86400)",
            [],
        )
        .with_context(|| format!("failed to seed SQLite database {}", path.display()))?;

        let defaults = RuntimeConfig::default();
        let config = conn
            .query_row(
                "SELECT max_messages, max_routes, max_peers, message_reset_secs
                 FROM runtime_config WHERE id = 1",
                [],
                |row| {
                    Ok(RuntimeConfig {
                        max_messages: row.get::<_, i64>(0)?.max(0) as usize,
                        max_routes: row.get::<_, i64>(1)?.max(0) as usize,
                        max_peers: row.get::<_, i64>(2)?.max(0) as usize,
                        message_reset_secs: row.get::<_, i64>(3)?.max(0) as usize,
                    })
                },
            )
            .optional()?
            .unwrap_or(defaults)
            .validate()
            .unwrap_or(defaults);

        Ok(Self {
            conn: Mutex::new(conn),
            config: Mutex::new(config),
        })
    }

    pub fn get(&self) -> RuntimeConfig {
        *self.config.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn update(&self, next: RuntimeConfig) -> anyhow::Result<RuntimeConfig> {
        let next = next.validate()?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE runtime_config
             SET max_messages = ?1, max_routes = ?2, max_peers = ?3,
                 message_reset_secs = ?4
             WHERE id = 1",
            params![
                next.max_messages as i64,
                next.max_routes as i64,
                next.max_peers as i64,
                next.message_reset_secs as i64
            ],
        )
        .context("failed to update runtime config")?;
        drop(conn);
        *self.config.lock().unwrap_or_else(|e| e.into_inner()) = next;
        Ok(next)
    }
}
