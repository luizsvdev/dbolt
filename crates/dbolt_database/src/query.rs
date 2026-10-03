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
