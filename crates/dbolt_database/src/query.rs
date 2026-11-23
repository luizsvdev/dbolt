use crate::DatabaseError;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    /// A value whose database type the driver cannot decode yet. Carries the
    /// type name so the UI can show something meaningful instead of failing.
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub type_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRequest {
    pub text: String,
    pub row_limit: Option<usize>,
}

impl QueryRequest {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            row_limit: None,
        }
    }

    pub fn with_row_limit(mut self, row_limit: usize) -> Self {
        self.row_limit = Some(row_limit);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct QueryResult {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
    /// Set when the driver stopped reading because `row_limit` was reached
    /// and more rows were available.
    pub truncated: bool,
}

/// Reads a query's rows a page at a time, so a large result never has to be
/// held in memory at once.
///
/// Every method blocks on network I/O and must run on a background task.
/// Dropping the cursor releases it on the server.
pub trait QueryCursor: Send {
    fn columns(&self) -> &[Column];

    /// Set for statements that change rows instead of returning them.
    fn rows_affected(&self) -> Option<u64>;

    /// Returns at most `max_rows` rows; an empty page means the cursor is
    /// exhausted.
    fn fetch(&mut self, max_rows: usize) -> Result<Vec<Vec<Value>>, DatabaseError>;
}

/// A cursor over a result that is already fully loaded, for drivers that
/// cannot page on the server.
pub struct MemoryCursor {
    columns: Vec<Column>,
    rows: std::vec::IntoIter<Vec<Value>>,
    rows_affected: Option<u64>,
}

impl MemoryCursor {
    pub fn new(result: QueryResult) -> Self {
        let rows_affected = result
            .columns
            .is_empty()
            .then_some(0);
        Self {
            columns: result.columns,
            rows: result.rows.into_iter(),
            rows_affected,
        }
    }
}

impl MemoryCursor {
    /// For statements that report a row count without returning rows.
    pub fn from_rows_affected(rows_affected: u64) -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new().into_iter(),
            rows_affected: Some(rows_affected),
        }
    }
}

impl QueryCursor for MemoryCursor {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn rows_affected(&self) -> Option<u64> {
        self.rows_affected
    }

    fn fetch(&mut self, max_rows: usize) -> Result<Vec<Vec<Value>>, DatabaseError> {
        Ok(self
            .rows
            .by_ref()
            .take(max_rows)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{Column, DatabaseError, MemoryCursor, QueryCursor, QueryResult, Value};

    #[test]
    fn memory_cursor_returns_pages_until_empty() -> Result<(), DatabaseError> {
        let mut cursor = MemoryCursor::new(QueryResult {
            columns: vec![Column {
                name: "id".to_string(),
                type_name: "int4".to_string(),
            }],
            rows: (0..5)
                .map(|id| vec![Value::Int(id)])
                .collect(),
            truncated: false,
        });
        assert_eq!(cursor.fetch(2)?.len(), 2);
        assert_eq!(cursor.fetch(2)?.len(), 2);
        assert_eq!(cursor.fetch(2)?.len(), 1);
        assert!(cursor.fetch(2)?.is_empty());
        assert_eq!(cursor.rows_affected(), None);
        Ok(())
    }
}
