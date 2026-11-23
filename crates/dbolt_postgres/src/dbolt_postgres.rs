use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use dbolt_database::{
    Column, ColumnKey, Connection, ConnectionForm, DatabaseCategory, DatabaseError, Driver,
    DriverCapabilities, DriverMetadata, FormField, FormFieldKind, FormValues, MemoryCursor,
    QueryCursor, QueryLanguage, QueryRequest, QueryResult, RelationalConnection, TableInfo,
    TableKind, Value,
};
use postgres::error::SqlState;
use postgres::fallible_iterator::FallibleIterator;
use postgres::types::{ToSql, Type};
use postgres::{Client, NoTls, Row, SimpleQueryMessage};

const DEFAULT_PORT: u16 = 5432;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PING_TIMEOUT: Duration = Duration::from_secs(5);

const FORM: [FormField; 5] = [
    FormField {
        id: "host",
        label: "Host",
        kind: FormFieldKind::Text,
        required: true,
        default: Some("localhost"),
    },
    FormField {
        id: "port",
        label: "Port",
        kind: FormFieldKind::Number,
        required: true,
        default: Some("5432"),
    },
    FormField {
        id: "database",
        label: "Database",
        kind: FormFieldKind::Text,
        required: false,
        default: None,
    },
    FormField {
        id: "user",
        label: "User",
        kind: FormFieldKind::Text,
        required: true,
        default: None,
    },
    FormField {
        id: "password",
        label: "Password",
        kind: FormFieldKind::Password,
        required: false,
        default: None,
    },
];

pub struct PostgresDriver {
    metadata: DriverMetadata,
}

impl PostgresDriver {
    pub fn new() -> Self {
        Self {
            metadata: DriverMetadata {
                id: "postgres",
                display_name: "PostgreSQL",
                category: DatabaseCategory::Relational,
                query_language: QueryLanguage::Sql,
                capabilities: DriverCapabilities::SCHEMAS,
                default_port: Some(DEFAULT_PORT),
            },
        }
    }
}

impl Default for PostgresDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl Driver for PostgresDriver {
    fn metadata(&self) -> &DriverMetadata {
        &self.metadata
    }

    fn connection_form(&self) -> &ConnectionForm {
        &FORM
    }

    fn connect(&self, values: &FormValues) -> Result<Box<dyn Connection>, DatabaseError> {
        let host = form_value(values, "host").unwrap_or("localhost");
        let port = match form_value(values, "port") {
            Some(text) => text
                .parse::<u16>()
                .map_err(|_| {
                    DatabaseError::InvalidConfig(format!("\"{text}\" is not a valid port"))
                })?,
            None => DEFAULT_PORT,
        };
        let user = form_value(values, "user")
            .ok_or_else(|| DatabaseError::InvalidConfig("user is required".to_string()))?;

        // Built field by field rather than from a connection string so the
        // password never ends up in a string that could be logged.
        let mut config = postgres::Config::new();
        config
            .host(host)
            .port(port)
            .user(user)
            .connect_timeout(CONNECT_TIMEOUT);
        if let Some(database) = form_value(values, "database") {
            config.dbname(database);
        }
        if let Some(password) = form_value(values, "password") {
            config.password(password);
        }

        // TLS is not wired up yet, so the SSL capability is not advertised.
        let client = config
            .connect(NoTls)
            .map_err(|error| DatabaseError::Connection(describe_error(&error)))?;

        Ok(Box::new(PostgresConnection {
            session: Arc::new(Mutex::new(Session {
                client,
                open_cursor: None,
                next_cursor_id: 0,
            })),
        }))
    }
}

fn form_value<'a>(values: &'a FormValues, id: &str) -> Option<&'a str> {
    values
        .get(id)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}

fn describe_error(error: &postgres::Error) -> String {
    match error.as_db_error() {
        Some(database_error) => database_error
            .message()
            .to_string(),
        None => match std::error::Error::source(error) {
            Some(cause) => format!("{error}: {cause}"),
            None => error.to_string(),
        },
    }
}

fn query_error(error: postgres::Error) -> DatabaseError {
    DatabaseError::Query(describe_error(&error))
}

struct PostgresConnection {
    session: Arc<Mutex<Session>>,
}

/// The client plus the server-side cursor currently open on it.
///
/// A cursor lives inside a transaction, and anything else run on the same
/// client would run inside that transaction too: its changes would only be
/// committed when the cursor closes, and one failing statement would abort the
/// cursor with it. So at most one cursor is open per connection, and it is
/// closed before any other statement runs. Its result keeps the rows it already
/// read; asking it for more reports that it was closed.
struct Session {
    client: Client,
    open_cursor: Option<OpenCursor>,
    next_cursor_id: u64,
}

struct OpenCursor {
    id: u64,
    /// False when the cursor was declared inside a transaction the user began
    /// themselves, which is left for them to commit or roll back.
    owns_transaction: bool,
}

const DECLARE_SAVEPOINT: &str = "dbolt_declare";

fn cursor_name(id: u64) -> String {
    format!("dbolt_cursor_{id}")
}

impl Session {
    fn close_cursor(&mut self) {
        let Some(cursor) = self.open_cursor.take() else {
            return;
        };
        let name = cursor_name(cursor.id);
        let close = if cursor.owns_transaction {
            // Committed rather than rolled back: run without a cursor, the
            // query would have committed whatever the functions it calls wrote.
            format!("CLOSE {name}; COMMIT")
        } else {
            format!("CLOSE {name}")
        };
        if let Err(error) = self
            .client
            .batch_execute(&close)
        {
            eprintln!("failed to close cursor {name}: {}", describe_error(&error));
            if cursor.owns_transaction {
                self.rollback();
            }
        }
    }

    fn rollback(&mut self) {
        if let Err(error) = self
            .client
            .batch_execute("ROLLBACK")
        {
            eprintln!("failed to roll back: {}", describe_error(&error));
        }
    }

    /// Declares a cursor for `statement`, a single statement that returns rows.
    /// On failure the session is left as it was found, so the caller can run
    /// the statement another way.
    fn declare_cursor(&mut self, statement: &str) -> Result<OpenCursor, postgres::Error> {
        // A savepoint only succeeds inside a transaction block. That tells
        // whether the user has a transaction open, and lets a failed declare be
        // undone without aborting it.
        let owns_transaction = match self
            .client
            .batch_execute(&format!("SAVEPOINT {DECLARE_SAVEPOINT}"))
        {
            Ok(()) => false,
            Err(error) if error.code() == Some(&SqlState::NO_ACTIVE_SQL_TRANSACTION) => {
                self.client
                    .batch_execute("BEGIN")?;
                true
            }
            Err(error) => return Err(error),
        };

        let id = self.next_cursor_id;
        self.next_cursor_id += 1;
        let declare = format!(
            "DECLARE {} NO SCROLL CURSOR FOR {}",
            cursor_name(id),
            trim_statement(statement)
        );
        // The extended protocol refuses several statements in one string, so
        // the statement text cannot add a second command after the cursor.
        let no_parameters: [&(dyn ToSql + Sync); 0] = [];
        let declared = self
            .client
            .execute(declare.as_str(), &no_parameters);
        let finish = match (owns_transaction, declared.is_ok()) {
            (true, true) => None,
            (true, false) => Some("ROLLBACK".to_string()),
            (false, true) => Some(format!("RELEASE SAVEPOINT {DECLARE_SAVEPOINT}")),
            (false, false) => Some(format!(
                "ROLLBACK TO SAVEPOINT {DECLARE_SAVEPOINT}; RELEASE SAVEPOINT {DECLARE_SAVEPOINT}"
            )),
        };
        if let Some(finish) = finish {
            self.client
                .batch_execute(&finish)?;
        }
        declared?;
        Ok(OpenCursor {
            id,
            owns_transaction,
        })
    }
}

/// Drops trailing semicolons so the statement can follow `DECLARE ... FOR`.
fn trim_statement(statement: &str) -> &str {
    statement.trim_end_matches(|character: char| character == ';' || character.is_whitespace())
}

fn lock_session(session: &Mutex<Session>) -> MutexGuard<'_, Session> {
    // A panic while holding the lock cannot leave the client in a state that
    // is unsafe to reuse, so recover instead of poisoning the tab.
    session
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

impl PostgresConnection {
    /// The session with any open cursor closed, ready for another statement.
    fn idle_session(&self) -> MutexGuard<'_, Session> {
        let mut session = lock_session(&self.session);
        session.close_cursor();
        session
    }
}

impl Connection for PostgresConnection {
    fn ping(&self) -> Result<(), DatabaseError> {
        // An empty query is harmless inside an open cursor's transaction, so
        // the cursor can stay open.
        lock_session(&self.session)
            .client
            .is_valid(PING_TIMEOUT)
            .map_err(|error| DatabaseError::Connection(describe_error(&error)))
    }

    fn execute(&self, request: &QueryRequest) -> Result<QueryResult, DatabaseError> {
        let mut session = self.idle_session();
        let client = &mut session.client;
        // Preparing first gives column metadata even when no rows come back.
        let statement = client
            .prepare(&request.text)
            .map_err(query_error)?;
        let columns = columns_of(&statement);

        let no_parameters: [&(dyn ToSql + Sync); 0] = [];
        let mut row_iter = client
            .query_raw(&statement, no_parameters)
            .map_err(query_error)?;

        let mut rows = Vec::new();
        let mut truncated = false;
        while let Some(row) = row_iter
            .next()
            .map_err(query_error)?
        {
            if request
                .row_limit
                .is_some_and(|limit| rows.len() >= limit)
            {
                truncated = true;
                break;
            }
            rows.push(decode_row(&row)?);
        }

        Ok(QueryResult {
            columns,
            rows,
            truncated,
        })
    }

    fn open_query(&self, text: &str) -> Result<Box<dyn QueryCursor>, DatabaseError> {
        let mut session = self.idle_session();
        // Preparing first gives column metadata, separates statements that
        // return rows from those that do not, and rejects several statements.
        let statement = session
            .client
            .prepare(text)
            .map_err(query_error)?;
        if statement.columns().is_empty() {
            let no_parameters: [&(dyn ToSql + Sync); 0] = [];
            let rows_affected = session
                .client
                .execute(&statement, &no_parameters)
                .map_err(query_error)?;
            return Ok(Box::new(MemoryCursor::from_rows_affected(rows_affected)));
        }

        let mut columns = columns_of(&statement);
        mark_key_columns(&mut session.client, &statement, &mut columns);
        let types: Vec<Type> = statement
            .columns()
            .iter()
            .map(|column| column.type_().clone())
            .collect();

        match session.declare_cursor(text) {
            Ok(cursor) => {
                let id = cursor.id;
                session.open_cursor = Some(cursor);
                Ok(Box::new(PostgresCursor {
                    session: self.session.clone(),
                    id,
                    columns,
                    types,
                    finished: false,
                }))
            }
            // Statements such as `SHOW`, `EXPLAIN` or `INSERT ... RETURNING`
            // return rows but cannot be declared as a cursor. Running them now
            // is safe because a failed declare never executed anything.
            Err(_) => {
                let messages = session
                    .client
                    .simple_query(text)
                    .map_err(query_error)?;
                Ok(Box::new(MemoryCursor::new(QueryResult {
                    columns,
                    rows: decode_text_rows(&messages, &types),
                    truncated: false,
                })))
            }
        }
    }

    fn as_relational(&self) -> Option<&dyn RelationalConnection> {
        Some(self)
    }
}

fn columns_of(statement: &postgres::Statement) -> Vec<Column> {
    statement
        .columns()
        .iter()
        .map(|column| Column {
            name: column.name().to_string(),
            type_name: column
                .type_()
                .name()
                .to_string(),
            key: None,
        })
        .collect()
}

const KEY_COLUMNS_QUERY: &str = "\
    SELECT c.conrelid, k.attnum, c.contype = 'p' \
    FROM pg_constraint c, unnest(c.conkey) AS k(attnum) \
    WHERE c.contype IN ('p', 'f') AND c.conrelid = ANY($1)";

/// Marks the result columns that come straight from a primary or foreign key
/// column. The marks only decorate the grid, so when the lookup fails the
/// result is shown without them rather than failing the query.
fn mark_key_columns(client: &mut Client, statement: &postgres::Statement, columns: &mut [Column]) {
    let sources: Vec<Option<(u32, i16)>> = statement
        .columns()
        .iter()
        .map(|column| {
            column
                .table_oid()
                .zip(column.column_id())
        })
        .collect();
    let mut tables: Vec<u32> = sources
        .iter()
        .flatten()
        .map(|(table, _)| *table)
        .collect();
    tables.sort_unstable();
    tables.dedup();
    if tables.is_empty() {
        return;
    }
    let rows = match client.query(KEY_COLUMNS_QUERY, &[&tables]) {
        Ok(rows) => rows,
        Err(error) => {
            eprintln!("could not look up key columns: {}", describe_error(&error));
            return;
        }
    };
    for row in rows {
        let (Ok(table), Ok(attribute), Ok(is_primary)) = (
            row.try_get::<_, u32>(0),
            row.try_get::<_, i16>(1),
            row.try_get::<_, bool>(2),
        ) else {
            continue;
        };
        let key = if is_primary {
            ColumnKey::Primary
        } else {
            ColumnKey::Foreign
        };
        for (column, source) in columns
            .iter_mut()
            .zip(&sources)
        {
            if *source == Some((table, attribute)) && column.key != Some(ColumnKey::Primary) {
                column.key = Some(key);
            }
        }
    }
}

/// Reads rows from a server-side cursor with `FETCH`.
struct PostgresCursor {
    session: Arc<Mutex<Session>>,
    id: u64,
    columns: Vec<Column>,
    types: Vec<Type>,
    finished: bool,
}

impl QueryCursor for PostgresCursor {
    fn columns(&self) -> &[Column] {
        &self.columns
    }

    fn rows_affected(&self) -> Option<u64> {
        None
    }

    fn fetch(&mut self, max_rows: usize) -> Result<Vec<Vec<Value>>, DatabaseError> {
        if self.finished || max_rows == 0 {
            return Ok(Vec::new());
        }
        let mut session = lock_session(&self.session);
        let is_open = session
            .open_cursor
            .as_ref()
            .is_some_and(|cursor| cursor.id == self.id);
        if !is_open {
            self.finished = true;
            return Err(DatabaseError::Query(
                "the rest of this result is no longer available because another statement \
                 ran on the same connection"
                    .to_string(),
            ));
        }

        // Fetched in the text format, which covers every type (dates, JSON,
        // numeric, arrays, ...) the way the server prints it.
        let fetch = format!("FETCH FORWARD {max_rows} FROM {}", cursor_name(self.id));
        match session
            .client
            .simple_query(&fetch)
        {
            Ok(messages) => {
                let rows = decode_text_rows(&messages, &self.types);
                if rows.len() < max_rows {
                    self.finished = true;
                    session.close_cursor();
                }
                Ok(rows)
            }
            Err(error) => {
                self.finished = true;
                // The error aborted the cursor's transaction. One the user began
                // stays aborted until they roll it back, as it would in psql.
                if let Some(cursor) = session.open_cursor.take()
                    && cursor.owns_transaction
                {
                    session.rollback();
                }
                Err(query_error(error))
            }
        }
    }
}

impl Drop for PostgresCursor {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // A result can be dropped on the UI thread, and closing needs the
        // session lock, which a long query on another thread may hold. Closing
        // from a thread of its own means the caller never waits.
        let session = self.session.clone();
        let id = self.id;
        let spawned = std::thread::Builder::new()
            .name("dbolt-close-cursor".to_string())
            .spawn(move || {
                let mut session = lock_session(&session);
                if session
                    .open_cursor
                    .as_ref()
                    .is_some_and(|cursor| cursor.id == id)
                {
                    session.close_cursor();
                }
            });
        if let Err(error) = spawned {
            eprintln!("failed to close cursor {}: {error}", cursor_name(id));
        }
    }
}

fn decode_text_rows(messages: &[SimpleQueryMessage], types: &[Type]) -> Vec<Vec<Value>> {
    messages
        .iter()
        .filter_map(|message| match message {
            SimpleQueryMessage::Row(row) => Some(
                (0..row.len())
                    .map(|index| decode_text(types.get(index), row.get(index)))
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

fn decode_text(column_type: Option<&Type>, text: Option<&str>) -> Value {
    let Some(text) = text else {
        return Value::Null;
    };
    let parsed = match column_type {
        Some(&Type::BOOL) => match text {
            "t" => Some(Value::Bool(true)),
            "f" => Some(Value::Bool(false)),
            _ => None,
        },
        Some(&Type::INT2 | &Type::INT4 | &Type::INT8 | &Type::OID) => text
            .parse()
            .ok()
            .map(Value::Int),
        Some(&Type::FLOAT4 | &Type::FLOAT8) => text
            .parse()
            .ok()
            .map(Value::Float),
        _ => None,
    };
    parsed.unwrap_or_else(|| Value::Text(text.to_string()))
}

impl RelationalConnection for PostgresConnection {
    fn list_schemas(&self) -> Result<Vec<String>, DatabaseError> {
        let rows = self
            .idle_session()
            .client
            .query(
                "SELECT schema_name FROM information_schema.schemata \
                 WHERE schema_name NOT LIKE 'pg\\_%' AND schema_name <> 'information_schema' \
                 ORDER BY schema_name",
                &[],
            )
            .map_err(query_error)?;

        rows.iter()
            .map(|row| {
                row.try_get::<_, String>(0)
                    .map_err(query_error)
            })
            .collect()
    }

    fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>, DatabaseError> {
        let rows = self
            .idle_session()
            .client
            .query(
                "SELECT table_name, table_type FROM information_schema.tables \
                 WHERE table_schema = $1 ORDER BY table_name",
                &[&schema],
            )
            .map_err(query_error)?;

        rows.iter()
            .map(|row| {
                let name: String = row
                    .try_get(0)
                    .map_err(query_error)?;
                let table_type: String = row
                    .try_get(1)
                    .map_err(query_error)?;
                let kind = if table_type == "VIEW" {
                    TableKind::View
                } else {
                    TableKind::Table
                };
                Ok(TableInfo { name, kind })
            })
            .collect()
    }
}

fn decode_row(row: &Row) -> Result<Vec<Value>, DatabaseError> {
    (0..row.columns().len())
        .map(|index| decode_cell(row, index))
        .collect()
}

fn decode_cell(row: &Row, index: usize) -> Result<Value, DatabaseError> {
    let column_type = match row.columns().get(index) {
        Some(column) => column.type_().clone(),
        None => return Ok(Value::Null),
    };

    let value = match column_type {
        Type::BOOL => row
            .try_get::<_, Option<bool>>(index)
            .map_err(query_error)?
            .map(Value::Bool),
        Type::INT2 => row
            .try_get::<_, Option<i16>>(index)
            .map_err(query_error)?
            .map(|number| Value::Int(number.into())),
        Type::INT4 => row
            .try_get::<_, Option<i32>>(index)
            .map_err(query_error)?
            .map(|number| Value::Int(number.into())),
        Type::INT8 => row
            .try_get::<_, Option<i64>>(index)
            .map_err(query_error)?
            .map(Value::Int),
        Type::OID => row
            .try_get::<_, Option<u32>>(index)
            .map_err(query_error)?
            .map(|number| Value::Int(number.into())),
        Type::FLOAT4 => row
            .try_get::<_, Option<f32>>(index)
            .map_err(query_error)?
            .map(|number| Value::Float(number.into())),
        Type::FLOAT8 => row
            .try_get::<_, Option<f64>>(index)
            .map_err(query_error)?
            .map(Value::Float),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => row
            .try_get::<_, Option<String>>(index)
            .map_err(query_error)?
            .map(Value::Text),
        Type::BYTEA => row
            .try_get::<_, Option<Vec<u8>>>(index)
            .map_err(query_error)?
            .map(Value::Bytes),
        other => {
            return Ok(Value::Unsupported(other.name().to_string()));
        }
    };

    Ok(value.unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use dbolt_database::Value;
    use postgres::types::Type;

    use super::{decode_text, trim_statement};

    #[test]
    fn trim_statement_drops_trailing_semicolons_and_whitespace() {
        assert_eq!(trim_statement("SELECT 1;\n ; "), "SELECT 1");
        assert_eq!(trim_statement("SELECT ';'"), "SELECT ';'");
    }

    #[test]
    fn decode_text_parses_known_types_and_keeps_the_rest_as_text() {
        assert_eq!(decode_text(Some(&Type::BOOL), Some("t")), Value::Bool(true));
        assert_eq!(decode_text(Some(&Type::INT8), Some("-42")), Value::Int(-42));
        assert_eq!(
            decode_text(Some(&Type::FLOAT8), Some("1.5")),
            Value::Float(1.5)
        );
        assert_eq!(
            decode_text(Some(&Type::TIMESTAMPTZ), Some("2026-01-01 00:00:00+00")),
            Value::Text("2026-01-01 00:00:00+00".to_string())
        );
        assert_eq!(decode_text(Some(&Type::INT4), None), Value::Null);
    }
}
