//! Local SQLite storage: cameras (without secrets), groups, settings, a cache of
//! recording indexes, and export history.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};

use crate::model::{Brand, CameraGroup, ExportJob};

/// A saved camera. Passwords live in the OS keychain, not here.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraRecord {
    pub id: String,
    pub name: String,
    pub brand: Brand,
    /// The camera's address on the LAN, or the cloud device id for Qubo cameras.
    pub host: String,
    pub model: Option<String>,
    pub firmware: Option<String>,
    pub mac: Option<String>,
    pub group_ids: Vec<String>,
    pub favorite: bool,
    pub has_camera_account: bool,
    /// Certificate fingerprint pinned on first connection (Tapo cameras).
    pub cert_pin: Option<String>,
    pub created_at: String,
}

const MIGRATIONS: &[&str] = &[
    // 1: initial schema
    "CREATE TABLE cameras (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        host TEXT NOT NULL,
        model TEXT,
        firmware TEXT,
        mac TEXT,
        group_ids TEXT NOT NULL DEFAULT '[]',
        favorite INTEGER NOT NULL DEFAULT 0,
        has_camera_account INTEGER NOT NULL DEFAULT 0,
        cert_pin TEXT,
        position INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL
    );
    CREATE TABLE camera_groups (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        position INTEGER NOT NULL
    );
    CREATE TABLE settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    CREATE TABLE day_cache (
        camera_id TEXT NOT NULL,
        date TEXT NOT NULL,
        json TEXT NOT NULL,
        fetched_at INTEGER NOT NULL,
        PRIMARY KEY (camera_id, date)
    );
    CREATE TABLE exports (
        id TEXT PRIMARY KEY,
        json TEXT NOT NULL,
        created_at TEXT NOT NULL
    );",
    // 2: Qubo cloud cameras
    "ALTER TABLE cameras ADD COLUMN brand TEXT NOT NULL DEFAULT 'tapo';",
];

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        Self::init(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        for (i, migration) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            conn.execute_batch(migration)?;
            conn.pragma_update(None, "user_version", (i + 1) as i64)?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> rusqlite::Result<T> {
        f(&self.conn.lock().expect("db lock"))
    }

    pub fn cameras(&self) -> rusqlite::Result<Vec<CameraRecord>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT id, name, host, model, firmware, mac, group_ids, favorite,
                        has_camera_account, cert_pin, created_at, brand
                 FROM cameras ORDER BY position, created_at",
            )?;
            stmt.query_map([], |row| {
                let groups: String = row.get(6)?;
                Ok(CameraRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    host: row.get(2)?,
                    model: row.get(3)?,
                    firmware: row.get(4)?,
                    mac: row.get(5)?,
                    group_ids: serde_json::from_str(&groups).unwrap_or_default(),
                    favorite: row.get(7)?,
                    has_camera_account: row.get(8)?,
                    cert_pin: row.get(9)?,
                    created_at: row.get(10)?,
                    brand: match row.get::<_, String>(11)?.as_str() {
                        "qubo" => Brand::Qubo,
                        _ => Brand::Tapo,
                    },
                })
            })?
            .collect()
        })
    }

    pub fn upsert_camera(&self, camera: &CameraRecord) -> rusqlite::Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO cameras (id, name, host, model, firmware, mac, group_ids, favorite,
                                      has_camera_account, cert_pin, created_at, position, brand)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                         (SELECT COALESCE(MAX(position), 0) + 1 FROM cameras), ?12)
                 ON CONFLICT(id) DO UPDATE SET
                     name = excluded.name, host = excluded.host, model = excluded.model,
                     firmware = excluded.firmware, mac = excluded.mac,
                     group_ids = excluded.group_ids, favorite = excluded.favorite,
                     has_camera_account = excluded.has_camera_account,
                     cert_pin = excluded.cert_pin, brand = excluded.brand",
                params![
                    camera.id,
                    camera.name,
                    camera.host,
                    camera.model,
                    camera.firmware,
                    camera.mac,
                    serde_json::to_string(&camera.group_ids).expect("JSON"),
                    camera.favorite,
                    camera.has_camera_account,
                    camera.cert_pin,
                    camera.created_at,
                    match camera.brand {
                        Brand::Tapo => "tapo",
                        Brand::Qubo => "qubo",
                    },
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_camera(&self, id: &str) -> rusqlite::Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM cameras WHERE id = ?1", [id])?;
            c.execute("DELETE FROM day_cache WHERE camera_id = ?1", [id])?;
            Ok(())
        })
    }

    pub fn groups(&self) -> rusqlite::Result<Vec<CameraGroup>> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT id, name FROM camera_groups ORDER BY position")?;
            stmt.query_map([], |row| {
                Ok(CameraGroup {
                    id: row.get(0)?,
                    name: row.get(1)?,
                })
            })?
            .collect()
        })
    }

    pub fn save_groups(&self, groups: &[CameraGroup]) -> rusqlite::Result<()> {
        let mut conn = self.conn.lock().expect("db lock");
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM camera_groups", [])?;
        for (position, group) in groups.iter().enumerate() {
            tx.execute(
                "INSERT INTO camera_groups (id, name, position) VALUES (?1, ?2, ?3)",
                params![group.id, group.name, position as i64],
            )?;
        }
        tx.commit()
    }

    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> rusqlite::Result<Option<T>> {
        let raw: Option<String> = self.with(|c| {
            c.query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
        })?;
        Ok(raw.and_then(|r| serde_json::from_str(&r).ok()))
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> rusqlite::Result<()> {
        let json = serde_json::to_string(value).expect("JSON");
        self.with(|c| {
            c.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, json],
            )?;
            Ok(())
        })
    }

    /// Cached day index JSON and the unix time it was fetched.
    pub fn cached_day(
        &self,
        camera_id: &str,
        date: &str,
    ) -> rusqlite::Result<Option<(String, i64)>> {
        self.with(|c| {
            c.query_row(
                "SELECT json, fetched_at FROM day_cache WHERE camera_id = ?1 AND date = ?2",
                params![camera_id, date],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
        })
    }

    pub fn cache_day(
        &self,
        camera_id: &str,
        date: &str,
        json: &str,
        fetched_at: i64,
    ) -> rusqlite::Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO day_cache (camera_id, date, json, fetched_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(camera_id, date) DO UPDATE SET json = excluded.json,
                                                           fetched_at = excluded.fetched_at",
                params![camera_id, date, json, fetched_at],
            )?;
            Ok(())
        })
    }

    pub fn exports(&self) -> rusqlite::Result<Vec<ExportJob>> {
        self.with(|c| {
            let mut stmt =
                c.prepare("SELECT json FROM exports ORDER BY created_at DESC LIMIT 200")?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            Ok(rows
                .filter_map(|r| r.ok())
                .filter_map(|json| serde_json::from_str(&json).ok())
                .collect())
        })
    }

    pub fn save_export(&self, job: &ExportJob) -> rusqlite::Result<()> {
        let json = serde_json::to_string(job).expect("JSON");
        self.with(|c| {
            c.execute(
                "INSERT INTO exports (id, json, created_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET json = excluded.json",
                params![job.id, json, job.created_at],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str) -> CameraRecord {
        CameraRecord {
            id: id.into(),
            name: "Gate".into(),
            brand: Brand::Tapo,
            host: "192.168.1.20".into(),
            model: Some("C325WB".into()),
            firmware: None,
            mac: None,
            group_ids: vec!["outdoor".into()],
            favorite: true,
            has_camera_account: false,
            cert_pin: Some("AB".into()),
            created_at: "2026-09-29T00:00:00Z".into(),
        }
    }

    #[test]
    fn cameras_round_trip() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_camera(&record("a")).unwrap();
        db.upsert_camera(&record("b")).unwrap();
        let mut renamed = record("a");
        renamed.name = "Front gate".into();
        db.upsert_camera(&renamed).unwrap();

        let cameras = db.cameras().unwrap();
        assert_eq!(cameras.len(), 2);
        assert_eq!(cameras[0], renamed);
        db.delete_camera("a").unwrap();
        assert_eq!(db.cameras().unwrap().len(), 1);
    }

    #[test]
    fn qubo_cameras_round_trip() {
        let db = Db::open_in_memory().unwrap();
        let mut qubo = record("q1");
        qubo.brand = Brand::Qubo;
        qubo.host = "cloud-camera-1".into();
        db.upsert_camera(&qubo).unwrap();
        let loaded = db.cameras().unwrap();
        assert_eq!(loaded[0].brand, Brand::Qubo);
        assert_eq!(loaded[0].host, "cloud-camera-1");
    }

    #[test]
    fn brand_migration_preserves_existing_tapo_cameras() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute("INSERT INTO cameras (id, name, host, created_at) VALUES ('old', 'Gate', '192.168.1.20', '2026-09-29')", []).unwrap();
        let db = Db::init(conn).unwrap();
        let cameras = db.cameras().unwrap();
        assert_eq!(cameras[0].id, "old");
        assert_eq!(cameras[0].brand, Brand::Tapo);
        assert_eq!(cameras[0].host, "192.168.1.20");
    }

    #[test]
    fn settings_and_cache() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.setting::<u32>("x").unwrap(), None);
        db.set_setting("x", &5u32).unwrap();
        assert_eq!(db.setting::<u32>("x").unwrap(), Some(5));

        db.cache_day("a", "2026-09-29", "{}", 10).unwrap();
        db.cache_day("a", "2026-09-29", "[]", 20).unwrap();
        assert_eq!(
            db.cached_day("a", "2026-09-29").unwrap(),
            Some(("[]".into(), 20))
        );
    }

    #[test]
    fn groups_keep_order() {
        let db = Db::open_in_memory().unwrap();
        let groups = vec![
            CameraGroup {
                id: "2".into(),
                name: "Outdoor".into(),
            },
            CameraGroup {
                id: "1".into(),
                name: "Indoor".into(),
            },
        ];
        db.save_groups(&groups).unwrap();
        let loaded = db.groups().unwrap();
        assert_eq!(loaded[0].name, "Outdoor");
        assert_eq!(loaded.len(), 2);
    }
}
