//! Read-only SQLite payload accounting, including every index.
use rusqlite::{Connection, OpenFlags};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os().nth(1).ok_or("expected database path")?;
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut query = conn.prepare(
        "SELECT name, SUM(payload), SUM(pgsize) FROM dbstat GROUP BY name ORDER BY name",
    )?;
    let tables = query
        .query_map([], |row| {
            Ok(serde_json::json!({
                "name": row.get::<_, String>(0)?,
                "stored_bytes": row.get::<_, i64>(1)?,
                "page_bytes": row.get::<_, i64>(2)?,
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let stored: u64 = tables
        .iter()
        .map(|row| row["stored_bytes"].as_u64().unwrap())
        .sum();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "file_bytes": std::fs::metadata(&path)?.len(), "stored_bytes": stored, "tables": tables,
            "definition": "SQLite dbstat cell payload across tables and indexes; excludes unused pages"
        }))?
    );
    Ok(())
}
