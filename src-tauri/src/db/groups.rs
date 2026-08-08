//! Groups CRUD operations

use rusqlite::{Connection, Transaction};
use std::sync::Mutex;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct AgentGroup {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub protocol: String,
    pub strategy: Option<String>,
    pub api_key: Option<String>,
    pub enabled: Option<bool>,
    pub created_at: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
pub struct GroupModelResource {
    pub provider_id: String,
    pub model: String,
}

pub struct GroupRepo;

impl GroupRepo {
    pub fn insert_tx(tx: &Transaction<'_>, group: &AgentGroup) -> Result<(), String> {
        tx.execute(
            "INSERT INTO agent_groups (id, name, description, protocol, strategy, api_key, enabled) \
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)",
            rusqlite::params![
                group.id,
                group.name,
                group.description,
                group.protocol,
                group.strategy,
                group.enabled
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn list_all(&self, conn: &Mutex<Connection>) -> Result<Vec<AgentGroup>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, protocol, strategy, api_key, enabled, created_at \
                 FROM agent_groups ORDER BY name",
            )
            .map_err(|e| e.to_string())?;

        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        let mut groups = Vec::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            groups.push(AgentGroup {
                id: row.get(0).map_err(|e| e.to_string())?,
                name: row.get(1).map_err(|e| e.to_string())?,
                description: row.get(2).map_err(|e| e.to_string())?,
                protocol: row.get(3).map_err(|e| e.to_string())?,
                strategy: row.get(4).map_err(|e| e.to_string())?,
                api_key: row.get(5).map_err(|e| e.to_string())?,
                enabled: row.get(6).map_err(|e| e.to_string())?,
                created_at: row.get(7).map_err(|e| e.to_string())?,
            });
        }
        Ok(groups)
    }

    pub fn get_by_id(
        &self,
        conn: &Mutex<Connection>,
        id: &str,
    ) -> Result<Option<AgentGroup>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, description, protocol, strategy, api_key, enabled, created_at \
                 FROM agent_groups WHERE id=?1",
            )
            .map_err(|e| e.to_string())?;

        let mut rows = stmt
            .query(rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        match rows.next().map_err(|e| e.to_string())? {
            Some(row) => Ok(Some(AgentGroup {
                id: row.get(0).map_err(|e| e.to_string())?,
                name: row.get(1).map_err(|e| e.to_string())?,
                description: row.get(2).map_err(|e| e.to_string())?,
                protocol: row.get(3).map_err(|e| e.to_string())?,
                strategy: row.get(4).map_err(|e| e.to_string())?,
                api_key: row.get(5).map_err(|e| e.to_string())?,
                enabled: row.get(6).map_err(|e| e.to_string())?,
                created_at: row.get(7).map_err(|e| e.to_string())?,
            })),
            None => Ok(None),
        }
    }

    pub fn create(&self, conn: &Mutex<Connection>, group: &AgentGroup) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO agent_groups (id, name, description, protocol, strategy, api_key, enabled) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                group.id, group.name, group.description, group.protocol,
                group.strategy, group.api_key, group.enabled
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn update(&self, conn: &Mutex<Connection>, group: &AgentGroup) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE agent_groups SET name=?1, description=?2, protocol=?3, strategy=?4, \
             api_key=?5, enabled=?6 WHERE id=?7",
            rusqlite::params![
                group.name,
                group.description,
                group.protocol,
                group.strategy,
                group.api_key,
                group.enabled,
                group.id
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn delete(&self, conn: &Mutex<Connection>, id: &str) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM agent_groups WHERE id=?1",
            rusqlite::params![id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM group_accounts WHERE group_id=?1",
            rusqlite::params![id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM group_model_accounts WHERE group_id=?1",
            rusqlite::params![id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_account_ids(
        &self,
        conn: &Mutex<Connection>,
        group_id: &str,
    ) -> Result<Vec<String>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT account_id FROM group_accounts WHERE group_id=?1 ORDER BY weight DESC")
            .map_err(|e| e.to_string())?;

        let mut rows = stmt
            .query(rusqlite::params![group_id])
            .map_err(|e| e.to_string())?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            ids.push(row.get::<_, String>(0).map_err(|e| e.to_string())?);
        }
        Ok(ids)
    }

    pub fn get_model_resources(
        &self,
        conn: &Mutex<Connection>,
        group_id: &str,
    ) -> Result<Vec<GroupModelResource>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT provider_id, model FROM group_model_resources \
                 WHERE group_id=?1 ORDER BY provider_id, model",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![group_id], |row| {
                Ok(GroupModelResource {
                    provider_id: row.get(0)?,
                    model: row.get(1)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn get_model_account_ids(
        &self,
        conn: &Mutex<Connection>,
        group_id: &str,
        provider_id: &str,
        model: &str,
    ) -> Result<Vec<String>, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT account_id FROM group_model_accounts
                 WHERE group_id=?1 AND provider_id=?2 AND model=?3
                 ORDER BY account_id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![group_id, provider_id, model], |row| {
                row.get(0)
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<String>, _>>()
            .map_err(|e| e.to_string())
    }

    pub fn set_model_account_ids(
        &self,
        conn: &Mutex<Connection>,
        group_id: &str,
        provider_id: &str,
        model: &str,
        account_ids: &[String],
    ) -> Result<(), String> {
        let mut conn = conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM group_model_accounts
             WHERE group_id=?1 AND provider_id=?2 AND model=?3",
            rusqlite::params![group_id, provider_id, model],
        )
        .map_err(|e| e.to_string())?;
        for account_id in account_ids {
            tx.execute(
                "INSERT INTO group_model_accounts (group_id, provider_id, model, account_id)
                 SELECT ?1, ?2, ?3, ?4
                 WHERE EXISTS (SELECT 1 FROM group_model_resources
                               WHERE group_id=?1 AND provider_id=?2 AND model=?3)",
                rusqlite::params![group_id, provider_id, model, account_id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn add_model_resources(
        &self,
        conn: &Mutex<Connection>,
        group_id: &str,
        resources: &[GroupModelResource],
    ) -> Result<usize, String> {
        let mut conn = conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let mut inserted = 0usize;
        for resource in resources {
            inserted += tx
                .execute(
                    "INSERT OR IGNORE INTO group_model_resources (group_id, provider_id, model) VALUES (?1, ?2, ?3)",
                    rusqlite::params![group_id, resource.provider_id, resource.model],
                )
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(inserted)
    }

    pub fn remove_model_resource(
        &self,
        conn: &Mutex<Connection>,
        group_id: &str,
        provider_id: &str,
        model: &str,
    ) -> Result<bool, String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        let affected = conn
            .execute(
                "DELETE FROM group_model_resources WHERE group_id=?1 AND provider_id=?2 AND model=?3",
                rusqlite::params![group_id, provider_id, model],
            )
            .map_err(|e| e.to_string())?;
        Ok(affected > 0)
    }

    pub fn set_model_resources(
        &self,
        conn: &Mutex<Connection>,
        group_id: &str,
        resources: &[GroupModelResource],
    ) -> Result<(), String> {
        let mut conn = conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM group_model_resources WHERE group_id=?1",
            rusqlite::params![group_id],
        )
        .map_err(|e| e.to_string())?;
        // The FK cascade removes exact account selections for deleted resources.
        for resource in resources {
            tx.execute(
                "INSERT INTO group_model_resources (group_id, provider_id, model) VALUES (?1, ?2, ?3)",
                rusqlite::params![group_id, resource.provider_id, resource.model],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    }

    pub fn add_account(
        &self,
        conn: &Mutex<Connection>,
        group_id: &str,
        account_id: &str,
        weight: i64,
    ) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO group_accounts (group_id, account_id, weight) VALUES (?1, ?2, ?3)",
            rusqlite::params![group_id, account_id, weight],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn remove_account(
        &self,
        conn: &Mutex<Connection>,
        group_id: &str,
        account_id: &str,
    ) -> Result<(), String> {
        let conn = conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM group_accounts WHERE group_id=?1 AND account_id=?2",
            rusqlite::params![group_id, account_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Mutex<Connection> {
        let conn = Connection::open_in_memory().expect("open database");
        conn.execute_batch(include_str!("../../migrations/001_initial.sql"))
            .expect("migrate base");
        conn.execute_batch(include_str!("../../migrations/008_client_keys.sql"))
            .expect("migrate client keys");
        conn.execute_batch(include_str!(
            "../../migrations/010_route_pool_management.sql"
        ))
        .expect("migrate route pool management");
        conn.execute_batch(include_str!(
            "../../migrations/012_group_model_accounts.sql"
        ))
        .expect("migrate model account mapping");
        conn.execute_batch(
            "INSERT INTO agent_groups (id, name, protocol) VALUES ('pool-a', 'Pool A', 'openai');
             INSERT INTO agent_groups (id, name, protocol) VALUES ('pool-b', 'Pool B', 'openai');
             INSERT INTO providers (id, name, type, base_url, protocol) VALUES ('provider-a', 'Provider A', 'official', 'https://a.example.com', 'openai');
             INSERT INTO providers (id, name, type, base_url, protocol) VALUES ('provider-b', 'Provider B', 'official', 'https://b.example.com', 'openai');",
        )
        .expect("insert fixtures");
        Mutex::new(conn)
    }

    #[test]
    fn add_model_resources_is_atomic_and_ignores_duplicates() {
        let db = test_db();
        let resources = vec![
            GroupModelResource {
                provider_id: "provider-a".into(),
                model: "shared-model".into(),
            },
            GroupModelResource {
                provider_id: "provider-a".into(),
                model: "model-two".into(),
            },
        ];

        assert_eq!(
            GroupRepo
                .add_model_resources(&db, "pool-a", &resources)
                .expect("add resources"),
            2
        );
        assert_eq!(
            GroupRepo
                .add_model_resources(&db, "pool-a", &resources)
                .expect("ignore duplicates"),
            0
        );
        assert_eq!(
            GroupRepo
                .get_model_resources(&db, "pool-a")
                .expect("list resources")
                .len(),
            2
        );
    }

    #[test]
    fn provider_model_identity_and_pool_scope_are_isolated() {
        let db = test_db();
        for (pool, provider) in [
            ("pool-a", "provider-a"),
            ("pool-a", "provider-b"),
            ("pool-b", "provider-a"),
        ] {
            GroupRepo
                .add_model_resources(
                    &db,
                    pool,
                    &[GroupModelResource {
                        provider_id: provider.into(),
                        model: "shared-model".into(),
                    }],
                )
                .expect("add scoped resource");
        }

        assert_eq!(
            GroupRepo
                .get_model_resources(&db, "pool-a")
                .expect("list first pool")
                .len(),
            2
        );
        assert_eq!(
            GroupRepo
                .get_model_resources(&db, "pool-b")
                .expect("list second pool")
                .len(),
            1
        );
    }

    #[test]
    fn exact_model_accounts_are_isolated_by_resource() {
        let db = test_db();
        {
            let conn = db.lock().expect("lock database");
            conn.execute_batch(
                "INSERT INTO accounts (id, provider_id, name, api_key, status)
                 VALUES ('account-a', 'provider-a', 'Account A', 'key-a', 'active');
                 INSERT INTO accounts (id, provider_id, name, api_key, status)
                 VALUES ('account-b', 'provider-a', 'Account B', 'key-b', 'active');",
            )
            .expect("insert accounts");
        }
        GroupRepo
            .add_model_resources(
                &db,
                "pool-a",
                &[GroupModelResource {
                    provider_id: "provider-a".into(),
                    model: "shared-model".into(),
                }],
            )
            .expect("add resource");
        GroupRepo
            .set_model_account_ids(
                &db,
                "pool-a",
                "provider-a",
                "shared-model",
                &["account-b".into()],
            )
            .expect("set exact accounts");

        assert_eq!(
            GroupRepo
                .get_model_account_ids(&db, "pool-a", "provider-a", "shared-model")
                .expect("get exact accounts"),
            vec!["account-b".to_string()]
        );
        assert!(GroupRepo
            .get_model_account_ids(&db, "pool-b", "provider-a", "shared-model")
            .expect("other pool remains empty")
            .is_empty());
    }

    #[test]
    fn remove_model_resource_is_precise_and_reports_missing() {
        let db = test_db();
        GroupRepo
            .add_model_resources(
                &db,
                "pool-a",
                &[
                    GroupModelResource {
                        provider_id: "provider-a".into(),
                        model: "shared-model".into(),
                    },
                    GroupModelResource {
                        provider_id: "provider-b".into(),
                        model: "shared-model".into(),
                    },
                ],
            )
            .expect("add resources");

        assert!(GroupRepo
            .remove_model_resource(&db, "pool-a", "provider-a", "shared-model")
            .expect("remove exact resource"));
        assert!(!GroupRepo
            .remove_model_resource(&db, "pool-a", "provider-a", "shared-model")
            .expect("report missing resource"));
        assert_eq!(
            GroupRepo
                .get_model_resources(&db, "pool-a")
                .expect("list remaining resources"),
            vec![GroupModelResource {
                provider_id: "provider-b".into(),
                model: "shared-model".into(),
            }]
        );
    }
}
