use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Params, Row, Statement};
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS clips (
    id              TEXT PRIMARY KEY,
    filename        TEXT NOT NULL,
    path            TEXT NOT NULL,
    duration        REAL NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    thumbnail_path  TEXT,
    tags            TEXT,
    folder          TEXT,
    upload_status   TEXT NOT NULL DEFAULT 'local',
    r2_key          TEXT,
    r2_url          TEXT,
    expiry_date     TEXT,
    is_permanent    INTEGER NOT NULL DEFAULT 0
);";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    pub filename: String,
    pub path: String,
    pub duration: f64,
    pub created_at: String,
    pub thumbnail_path: Option<String>,
    pub tags: Option<String>,
    pub folder: Option<String>,
    pub upload_status: String,
    pub r2_key: Option<String>,
    pub r2_url: Option<String>,
    pub expiry_date: Option<String>,
    pub is_permanent: bool,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn insert_clip(&self, c: &Clip) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO clips
             (id, filename, path, duration, created_at, thumbnail_path, tags, folder,
              upload_status, r2_key, r2_url, expiry_date, is_permanent)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![c.id, c.filename, c.path, c.duration, c.created_at, c.thumbnail_path,
                    c.tags, c.folder, c.upload_status, c.r2_key, c.r2_url, c.expiry_date,
                    c.is_permanent as i32],
        )?;
        Ok(())
    }

    pub fn get_all_clips(&self) -> rusqlite::Result<Vec<Clip>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM clips WHERE upload_status != 'deleted' ORDER BY created_at DESC"
        )?;
        collect(&mut stmt, [])
    }

    pub fn get_clips_by_folder(&self, folder: &str) -> rusqlite::Result<Vec<Clip>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM clips WHERE folder = ?1 ORDER BY created_at DESC"
        )?;
        collect(&mut stmt, [folder])
    }

    pub fn get_uploaded_clips(&self, permanent: bool) -> rusqlite::Result<Vec<Clip>> {
        let mut stmt = self.conn.prepare(
            "SELECT * FROM clips WHERE upload_status = 'uploaded' AND is_permanent = ?1
             ORDER BY created_at DESC"
        )?;
        collect(&mut stmt, [permanent as i32])
    }

    pub fn get_clip(&self, id: &str) -> rusqlite::Result<Clip> {
        self.conn.query_row("SELECT * FROM clips WHERE id = ?1", [id], row_to_clip)
    }

    pub fn update_clip_tags(&self, id: &str, tags: &str) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE clips SET tags = ?1 WHERE id = ?2", params![tags, id])?;
        Ok(())
    }

    pub fn update_clip_folder(&self, id: &str, folder: &str) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE clips SET folder = ?1 WHERE id = ?2", params![folder, id])?;
        Ok(())
    }

    pub fn mark_uploaded(&self, id: &str, url: &str, permanent: bool, expiry: Option<DateTime<Utc>>) -> rusqlite::Result<()> {
        let expiry = expiry.map(|e| e.to_rfc3339());
        let key = url.rsplit('/').next().unwrap_or(url);
        self.conn.execute(
            "UPDATE clips SET upload_status = 'uploaded', r2_url = ?1, r2_key = ?2,
             is_permanent = ?3, expiry_date = ?4 WHERE id = ?5",
            params![url, key, permanent as i32, expiry, id],
        )?;
        Ok(())
    }

    pub fn mark_deleted(&self, id: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE clips SET upload_status = 'deleted', r2_key = NULL, r2_url = NULL WHERE id = ?1",
            [id],
        )?;
        Ok(())
    }

    pub fn delete_clip(&self, id: &str) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM clips WHERE id = ?1", [id])?;
        Ok(())
    }
}

fn collect<P: Params>(stmt: &mut Statement, params: P) -> rusqlite::Result<Vec<Clip>> {
    stmt.query_map(params, row_to_clip)?.collect()
}

fn row_to_clip(row: &Row) -> rusqlite::Result<Clip> {
    Ok(Clip {
        id: row.get(0)?,
        filename: row.get(1)?,
        path: row.get(2)?,
        duration: row.get(3)?,
        created_at: row.get(4)?,
        thumbnail_path: row.get(5)?,
        tags: row.get(6)?,
        folder: row.get(7)?,
        upload_status: row.get(8)?,
        r2_key: row.get(9)?,
        r2_url: row.get(10)?,
        expiry_date: row.get(11)?,
        is_permanent: row.get::<_, i32>(12)? != 0,
    })
}
