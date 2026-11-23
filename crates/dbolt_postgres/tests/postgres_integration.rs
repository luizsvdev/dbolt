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

#[test]
#[ignore = "needs a running PostgreSQL server"]
fn open_query_reads_a_large_result_page_by_page() {
    let connection = connect();
    let mut cursor = connection
        .open_query("SELECT n, n::text AS label FROM generate_series(1, 2500) AS n;")
        .expect("open query");

    assert_eq!(cursor.columns().len(), 2);
    assert_eq!(cursor.rows_affected(), None);
    let first_page = cursor
        .fetch(1000)
        .expect("first page");
    assert_eq!(first_page.len(), 1000);
    assert_eq!(first_page[0], [Value::Int(1), Value::Text("1".to_string())]);

    let mut total = first_page.len();
    loop {
        let page = cursor
            .fetch(1000)
            .expect("page");
        if page.is_empty() {
            break;
        }
        total += page.len();
    }
    assert_eq!(total, 2500);
}

#[test]
#[ignore = "needs a running PostgreSQL server"]
fn open_query_decodes_values_the_binary_decoder_does_not_know_as_text() {
    let mut cursor = connect()
        .open_query(
            "SELECT '{\"a\": 1}'::jsonb AS document, 1.50::numeric AS amount, NULL::date AS day",
        )
        .expect("open query");

    let rows = cursor
        .fetch(10)
        .expect("rows");
    assert_eq!(
        rows,
        [vec![
            Value::Text("{\"a\": 1}".to_string()),
            Value::Text("1.50".to_string()),
            Value::Null,
        ]]
    );
}

#[test]
#[ignore = "needs a running PostgreSQL server"]
fn open_query_reports_rows_affected_for_statements_without_rows() {
    let connection = connect();
    connection
        .open_query("CREATE TEMP TABLE dbolt_affected (id int)")
        .expect("create");
    let cursor = connection
        .open_query("INSERT INTO dbolt_affected SELECT generate_series(1, 3)")
        .expect("insert");

    assert!(cursor.columns().is_empty());
    assert_eq!(cursor.rows_affected(), Some(3));
}

#[test]
#[ignore = "needs a running PostgreSQL server"]
fn open_query_falls_back_for_statements_that_cannot_be_a_cursor() {
    let connection = connect();
    connection
        .open_query("CREATE TEMP TABLE dbolt_returning (id int)")
        .expect("create");

    let mut cursor = connection
        .open_query("INSERT INTO dbolt_returning VALUES (7), (8) RETURNING id")
        .expect("insert returning");
    assert_eq!(
        cursor
            .fetch(10)
            .expect("rows"),
        [vec![Value::Int(7)], vec![Value::Int(8)]]
    );

    let mut cursor = connection
        .open_query("SHOW server_version")
        .expect("show");
    assert_eq!(
        cursor
            .fetch(10)
            .expect("rows")
            .len(),
        1
    );
}

#[test]
#[ignore = "needs a running PostgreSQL server"]
fn another_statement_closes_the_open_cursor() {
    let connection = connect();
    let mut cursor = connection
        .open_query("SELECT generate_series(1, 100)")
        .expect("open query");
    cursor
        .fetch(10)
        .expect("first page");

    let schemas = connection
        .as_relational()
        .expect("relational connection")
        .list_schemas()
        .expect("schemas");
    assert!(!schemas.is_empty());

    assert!(matches!(cursor.fetch(10), Err(DatabaseError::Query(_))));
}

#[test]
#[ignore = "needs a running PostgreSQL server"]
fn a_dropped_cursor_leaves_the_connection_usable() {
    let connection = connect();
    let mut cursor = connection
        .open_query("SELECT generate_series(1, 100)")
        .expect("open query");
    cursor
        .fetch(10)
        .expect("first page");
    drop(cursor);

    let mut cursor = connection
        .open_query("SELECT 1")
        .expect("open query after drop");
    assert_eq!(
        cursor
            .fetch(10)
            .expect("rows"),
        [vec![Value::Int(1)]]
    );
}

#[test]
#[ignore = "needs a running PostgreSQL server"]
fn a_cursor_inside_the_users_transaction_does_not_commit_it() {
    let connection = connect();
    connection
        .open_query("BEGIN")
        .expect("begin");
    connection
        .open_query("CREATE TEMP TABLE dbolt_uncommitted (id int)")
        .expect("create");
    let mut cursor = connection
        .open_query("SELECT * FROM dbolt_uncommitted")
        .expect("select inside the transaction");
    assert!(
        cursor
            .fetch(10)
            .expect("rows")
            .is_empty()
    );
    // A declare that fails inside the user's transaction must not abort it.
    connection
        .open_query("SHOW server_version")
        .expect("show inside the transaction");
    connection
        .open_query("ROLLBACK")
        .expect("rollback");

    let result = connection.open_query("SELECT * FROM dbolt_uncommitted");
    assert!(matches!(result, Err(DatabaseError::Query(_))));
}

#[test]
#[ignore = "needs a running PostgreSQL server"]
fn open_query_reports_sql_errors_as_query_errors() {
    let result = connect().open_query("SELECT * FROM table_that_does_not_exist");

    assert!(matches!(result, Err(DatabaseError::Query(_))));
}
