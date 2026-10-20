// To change the schema add migrations/NNNN_name.sql and one line to MIGRATIONS; never edit an applied migration.
//! Numbered schema migrations, tracked with SQLite's `PRAGMA user_version`. Each migration is applied once, in a
//! transaction. Migration 0001 is entirely `IF NOT EXISTS`, so databases created before the runner existed
//! (schema present, `user_version` 0) are adopted without change.
use rusqlite::Connection;

const MIGRATIONS: &[(&str, &str)] = &[("0001_init", include_str!("migrations/0001_init.sql")), ("0004_content", include_str!("migrations/0004_content.sql"))];

pub fn run(c: &Connection) -> rusqlite::Result<()> {
    let applied: usize = c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?.max(0) as usize;
    for (i, (_, sql)) in MIGRATIONS.iter().enumerate().skip(applied) {
        c.execute_batch(&format!("BEGIN;\n{sql}\nPRAGMA user_version = {};\nCOMMIT;", i + 1))
            .inspect_err(|_| { let _ = c.execute_batch("ROLLBACK"); })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrations_are_idempotent_and_versioned() {
        let c = Connection::open_in_memory().unwrap();
        run(&c).unwrap();
        run(&c).unwrap();
        let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v as usize, MIGRATIONS.len());
    }
}
