//! Tests marked `#[ignore]` need a running PostgreSQL server. Provide it via
//! DBOLT_TEST_POSTGRES_{HOST,PORT,DATABASE,USER,PASSWORD} and run
//! `cargo test -p dbolt_postgres -- --ignored`.

use dbolt_database::{
    Connection, DatabaseError, Driver, FormValues, QueryRequest, TableKind, Value,
};
use dbolt_postgres::PostgresDriver;

fn values_from_environment() -> FormValues {
    let mut values = FormValues::new();
    for (field, variable) in [
        ("host", "DBOLT_TEST_POSTGRES_HOST"),
        ("port", "DBOLT_TEST_POSTGRES_PORT"),
        ("database", "DBOLT_TEST_POSTGRES_DATABASE"),
        ("user", "DBOLT_TEST_POSTGRES_USER"),
        ("password", "DBOLT_TEST_POSTGRES_PASSWORD"),
    ] {
        if let Ok(value) = std::env::var(variable) {
            values.insert(field.to_string(), value);
        }
    }
    values
}

fn connect() -> Box<dyn Connection> {
    PostgresDriver::new()
        .connect(&values_from_environment())
        .expect("connect using DBOLT_TEST_POSTGRES_* variables")
}

#[test]
fn connecting_to_a_closed_port_reports_a_connection_error() {
    let values = FormValues::from([
        ("host".to_string(), "127.0.0.1".to_string()),
        ("port".to_string(), "1".to_string()),
        ("user".to_string(), "nobody".to_string()),
    ]);

    let result = PostgresDriver::new().connect(&values);

    assert!(matches!(result, Err(DatabaseError::Connection(_))));
}

#[test]
fn connecting_without_a_user_is_an_invalid_configuration() {
    let result = PostgresDriver::new().connect(&FormValues::new());

    assert!(matches!(result, Err(DatabaseError::InvalidConfig(_))));
}

#[test]
fn connecting_with_a_bad_port_is_an_invalid_configuration() {
    let values = FormValues::from([
        ("user".to_string(), "nobody".to_string()),
        ("port".to_string(), "not-a-port".to_string()),
    ]);

    let result = PostgresDriver::new().connect(&values);

    assert!(matches!(result, Err(DatabaseError::InvalidConfig(_))));
}

#[test]
#[ignore = "needs a running PostgreSQL server"]
fn ping_succeeds_on_a_live_connection() {
    connect()
        .ping()
        .expect("ping");
}

#[test]
#[ignore = "needs a running PostgreSQL server"]
fn execute_decodes_common_types_and_nulls() {
    let result = connect()
        .execute(&QueryRequest::new(
            "SELECT 1::int4 AS number, 'abc'::text AS word, true AS flag, \
             1.5::float8 AS ratio, NULL::text AS nothing, now() AS moment",
        ))
        .expect("execute");

    let names: Vec<&str> = result
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["number", "word", "flag", "ratio", "nothing", "moment"]
    );
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0][..5],
        [
            Value::Int(1),
            Value::Text("abc".to_string()),
            Value::Bool(true),
            Value::Float(1.5),
            Value::Null,
        ]
    );
    assert!(matches!(result.rows[0][5], Value::Unsupported(_)));
}

#[test]
#[ignore = "needs a running PostgreSQL server"]
fn execute_stops_at_the_row_limit_and_marks_the_result_truncated() {
    let result = connect()
        .execute(&QueryRequest::new("SELECT generate_series(1, 10)").with_row_limit(3))
        .expect("execute");

    assert_eq!(result.rows.len(), 3);
    assert!(result.truncated);
}

#[test]
#[ignore = "needs a running PostgreSQL server"]
fn execute_reports_sql_errors_as_query_errors() {
    let result = connect().execute(&QueryRequest::new(
        "SELECT * FROM table_that_does_not_exist",
    ));

    match result {
        Err(DatabaseError::Query(message)) => assert!(message.contains("does not exist")),
        other => panic!("expected a query error, got {other:?}"),
    }
}

#[test]
#[ignore = "needs a running PostgreSQL server"]
fn relational_browsing_lists_schemas_and_tables() {
    let connection = connect();
    let relational = connection
        .as_relational()
        .expect("relational connection");

    let schemas = relational
        .list_schemas()
        .expect("schemas");
    assert!(
        schemas
            .iter()
            .any(|schema| schema == "public")
    );

    let tables = relational
        .list_tables("public")
        .expect("tables");
    for table in &tables {
        println!(
            "{} {}",
            match table.kind {
                TableKind::Table => "table",
                TableKind::View => "view",
            },
            table.name
        );
    }
}
