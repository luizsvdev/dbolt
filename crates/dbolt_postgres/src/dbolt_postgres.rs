use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use dbolt_database::{
    Column, Connection, ConnectionForm, DatabaseCategory, DatabaseError, Driver,
    DriverCapabilities, DriverMetadata, FormField, FormFieldKind, FormValues, QueryLanguage,
    QueryRequest, QueryResult, RelationalConnection, TableInfo, TableKind, Value,
};
use postgres::fallible_iterator::FallibleIterator;
use postgres::types::{ToSql, Type};
use postgres::{Client, NoTls, Row};

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
            Some(text) => text.parse::<u16>().map_err(|_| {
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
            client: Mutex::new(client),
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
        Some(database_error) => database_error.message().to_string(),
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
    client: Mutex<Client>,
}

impl PostgresConnection {
    fn client(&self) -> MutexGuard<'_, Client> {
        // A panic while holding the lock cannot leave the client in a state
        // that is unsafe to reuse, so recover instead of poisoning the tab.
        self.client.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Connection for PostgresConnection {
    fn ping(&self) -> Result<(), DatabaseError> {
        self.client()
            .is_valid(PING_TIMEOUT)
            .map_err(|error| DatabaseError::Connection(describe_error(&error)))
    }

    fn execute(&self, request: &QueryRequest) -> Result<QueryResult, DatabaseError> {
        let mut client = self.client();
        // Preparing first gives column metadata even when no rows come back.
        let statement = client.prepare(&request.text).map_err(query_error)?;
        let columns: Vec<Column> = statement
            .columns()
            .iter()
            .map(|column| Column {
                name: column.name().to_string(),
                type_name: column.type_().name().to_string(),
            })
            .collect();

        let no_parameters: [&(dyn ToSql + Sync); 0] = [];
        let mut row_iter = client
            .query_raw(&statement, no_parameters)
            .map_err(query_error)?;

        let mut rows = Vec::new();
        let mut truncated = false;
        while let Some(row) = row_iter.next().map_err(query_error)? {
            if request.row_limit.is_some_and(|limit| rows.len() >= limit) {
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

    fn as_relational(&self) -> Option<&dyn RelationalConnection> {
        Some(self)
    }
}

impl RelationalConnection for PostgresConnection {
    fn list_schemas(&self) -> Result<Vec<String>, DatabaseError> {
        let rows = self
            .client()
            .query(
                "SELECT schema_name FROM information_schema.schemata \
                 WHERE schema_name NOT LIKE 'pg\\_%' AND schema_name <> 'information_schema' \
                 ORDER BY schema_name",
                &[],
            )
            .map_err(query_error)?;

        rows.iter()
            .map(|row| row.try_get::<_, String>(0).map_err(query_error))
            .collect()
    }

    fn list_tables(&self, schema: &str) -> Result<Vec<TableInfo>, DatabaseError> {
        let rows = self
            .client()
            .query(
                "SELECT table_name, table_type FROM information_schema.tables \
                 WHERE table_schema = $1 ORDER BY table_name",
                &[&schema],
            )
            .map_err(query_error)?;

        rows.iter()
            .map(|row| {
                let name: String = row.try_get(0).map_err(query_error)?;
                let table_type: String = row.try_get(1).map_err(query_error)?;
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
        other => return Ok(Value::Unsupported(other.name().to_string())),
    };

    Ok(value.unwrap_or(Value::Null))
}
