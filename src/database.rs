use rusqlite::{Connection, Error as SqliteError};

#[derive(Debug)]
pub enum DatabaseError {
    Connection(SqliteError),
    Query(SqliteError),
}

impl std::fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatabaseError::Connection(e) => write!(f, "Database connection error: {}", e),
            DatabaseError::Query(e) => write!(f, "Query error: {}", e),
        }
    }
}

impl std::error::Error for DatabaseError {}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(db_path: &str) -> Result<Self, DatabaseError> {
        let conn = Connection::open(db_path).map_err(DatabaseError::Connection)?;
        Ok(Self { conn })
    }

    pub fn init_schema(&self) -> Result<(), DatabaseError> {
        self.conn
            .execute_batch(
                "PRAGMA journal_mode=WAL;",
            )
            .map_err(DatabaseError::Query)
    }
}
