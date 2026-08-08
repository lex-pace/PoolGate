pub mod accounts;
pub mod client_keys;
pub mod groups;
pub mod logs;
pub mod providers;
pub mod schema;
pub mod settings;

use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

pub struct Database {
    pub conn: Mutex<Connection>,
    pub providers: providers::ProviderRepo,
    pub accounts: accounts::AccountRepo,
    pub groups: groups::GroupRepo,
    pub logs: logs::LogRepo,
    pub settings: settings::SettingsRepo,
    pub client_keys: client_keys::ClientKeyRepo,
}

impl Database {
    pub fn new(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let conn = Mutex::new(conn);
        Ok(Self {
            providers: providers::ProviderRepo,
            accounts: accounts::AccountRepo,
            groups: groups::GroupRepo,
            logs: logs::LogRepo,
            settings: settings::SettingsRepo,
            client_keys: client_keys::ClientKeyRepo,
            conn,
        })
    }

    pub fn run_migrations(&self) -> Result<(), Box<dyn std::error::Error>> {
        schema::run_migrations(&self.conn)?;
        Ok(())
    }
}
