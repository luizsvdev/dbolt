use std::sync::Arc;
use std::time::{Duration, Instant};

use dbolt_database::{Column, Connection, ConnectionId, QueryCursor, Value};
use gpui_kit::{AppContext, Context, EventEmitter, SharedString, Task};

use crate::util::ResultExt;

pub const PAGE_SIZE: usize = 500;

#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionState {
    Running,
    Ready,
    Failed(SharedString),
    Cancelled,
}

#[derive(Debug, Clone)]
pub enum QueryExecutionEvent {
    Finished,
    RowsLoaded,
    Failed(SharedString),
    Cancelled,
}

/// One run of one statement. The editor tab that started it holds it so that
/// closing the tab can cancel it; the results panel and console observe it.
pub struct QueryExecution {
    connection_id: ConnectionId,
    connection_name: SharedString,
    statement: SharedString,
    started_at: Instant,
    duration: Option<Duration>,
    state: ExecutionState,
    columns: Vec<Column>,
    rows: Vec<Vec<Value>>,
    rows_affected: Option<u64>,
    has_more: bool,
    /// Taken out while a background task reads from it and put back when the
    /// page arrives, so only one fetch can run at a time.
    cursor: Option<Box<dyn QueryCursor>>,
    loading_more: bool,
    task: Task<()>,
}

impl EventEmitter<QueryExecutionEvent> for QueryExecution {}

impl QueryExecution {
    pub fn start(
        connection: Arc<dyn Connection>,
        connection_id: ConnectionId,
        connection_name: SharedString,
        statement: SharedString,
        cx: &mut Context<Self>,
    ) -> Self {
        let task = cx.spawn({
            let statement = statement.clone();
            async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        let mut cursor = connection
                            .open_query(&statement)
                            .map_err(|error| error.to_string())?;
                        let rows = cursor
                            .fetch(PAGE_SIZE)
                            .map_err(|error| error.to_string())?;
                        Ok::<_, String>((cursor, rows))
                    })
                    .await;
                this.update(cx, |this, cx| this.finish_page(result, cx))
                    .log_err();
            }
        });
        Self {
            connection_id,
            connection_name,
            statement,
            started_at: Instant::now(),
            duration: None,
            state: ExecutionState::Running,
            columns: Vec::new(),
            rows: Vec::new(),
            rows_affected: None,
            has_more: false,
            cursor: None,
            loading_more: false,
            task,
        }
    }

    pub fn connection_id(&self) -> ConnectionId {
        self.connection_id
    }

    pub fn connection_name(&self) -> &SharedString {
        &self.connection_name
    }

    pub fn statement(&self) -> &SharedString {
        &self.statement
    }

    pub fn started_at(&self) -> Instant {
        self.started_at
    }

    /// Time until the first page arrived or the statement failed.
    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    pub fn state(&self) -> &ExecutionState {
        &self.state
    }

    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub fn rows(&self) -> &[Vec<Value>] {
        &self.rows
    }

    pub fn rows_affected(&self) -> Option<u64> {
        self.rows_affected
    }

    pub fn has_more(&self) -> bool {
        self.has_more
    }

    pub fn is_loading_more(&self) -> bool {
        self.loading_more
    }

    pub fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.loading_more || !self.has_more {
            return;
        }
        let Some(mut cursor) = self.cursor.take() else {
            return;
        };
        self.loading_more = true;
        self.task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let rows = cursor
                        .fetch(PAGE_SIZE)
                        .map_err(|error| error.to_string())?;
                    Ok::<_, String>((cursor, rows))
                })
                .await;
            this.update(cx, |this, cx| this.finish_page(result, cx))
                .log_err();
        });
        cx.notify();
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.state != ExecutionState::Running && !self.loading_more {
            return;
        }
        self.task = Task::ready(());
        self.cursor = None;
        self.has_more = false;
        self.loading_more = false;
        self.state = ExecutionState::Cancelled;
        self.duration
            .get_or_insert_with(|| self.started_at.elapsed());
        cx.emit(QueryExecutionEvent::Cancelled);
        cx.notify();
    }

    fn finish_page(
        &mut self,
        result: Result<(Box<dyn QueryCursor>, Vec<Vec<Value>>), String>,
        cx: &mut Context<Self>,
    ) {
        let first_page = self.state == ExecutionState::Running;
        self.loading_more = false;
        self.duration
            .get_or_insert_with(|| self.started_at.elapsed());
        match result {
            Ok((cursor, rows)) => {
                if first_page {
                    self.columns = cursor.columns().to_vec();
                    self.rows_affected = cursor.rows_affected();
                    self.state = ExecutionState::Ready;
                }
                self.has_more = rows.len() == PAGE_SIZE;
                self.rows.extend(rows);
                self.cursor = self
                    .has_more
                    .then_some(cursor);
                cx.emit(if first_page {
                    QueryExecutionEvent::Finished
                } else {
                    QueryExecutionEvent::RowsLoaded
                });
            }
            Err(message) => {
                let message = SharedString::from(message);
                self.has_more = false;
                self.state = ExecutionState::Failed(message.clone());
                cx.emit(QueryExecutionEvent::Failed(message));
            }
        }
        cx.notify();
    }
}
