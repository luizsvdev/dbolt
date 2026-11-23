use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dbolt_database::{Column, Connection, QueryCursor, Value};
use gpui_kit::{AppContext, Context, Entity, EventEmitter, SharedString, Subscription, Task};

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
    Failed,
    Cancelled,
}

type PageResult = Result<(Box<dyn QueryCursor>, Vec<Vec<Value>>), String>;

/// One run of one statement, started by a [`QueryRun`].
pub struct QueryExecution {
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

    pub fn statement(&self) -> &SharedString {
        &self.statement
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

    fn finish_page(&mut self, result: PageResult, cx: &mut Context<Self>) {
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
                self.state = ExecutionState::Failed(message);
                cx.emit(QueryExecutionEvent::Failed);
            }
        }
        cx.notify();
    }
}

/// Runs statements one after another on one connection, stopping at the
/// first one that fails. The editor tab that started it holds it so that
/// closing the tab can cancel it; the results panel observes it.
///
/// Observers read the state instead of receiving events, because the first
/// statement starts before anyone can subscribe to the new run.
pub struct QueryRun {
    connection: Arc<dyn Connection>,
    pending: VecDeque<SharedString>,
    executions: Vec<Entity<QueryExecution>>,
    /// How many of `executions`, from the start, have ended: returned their
    /// first page, failed or been cancelled.
    ended_count: usize,
    started_at: Instant,
    duration: Option<Duration>,
    _current_subscription: Option<Subscription>,
}

impl QueryRun {
    pub fn new(
        connection: Arc<dyn Connection>,
        statements: Vec<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut run = Self {
            connection,
            pending: statements.into(),
            executions: Vec::new(),
            ended_count: 0,
            started_at: Instant::now(),
            duration: None,
            _current_subscription: None,
        };
        run.start_next(cx);
        run
    }

    pub fn is_running(&self) -> bool {
        self.duration.is_none()
    }

    /// Wall time of the whole run, set once it ends.
    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    pub fn executions(&self) -> &[Entity<QueryExecution>] {
        &self.executions
    }

    pub fn ended_count(&self) -> usize {
        self.ended_count
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.pending.clear();
        match self.current() {
            Some(execution) => execution.update(cx, |execution, cx| execution.cancel(cx)),
            None => self.finish(cx),
        }
    }

    fn current(&self) -> Option<Entity<QueryExecution>> {
        self.executions
            .get(self.ended_count)
            .cloned()
    }

    fn start_next(&mut self, cx: &mut Context<Self>) {
        let Some(statement) = self.pending.pop_front() else {
            self.finish(cx);
            return;
        };
        let connection = self.connection.clone();
        let execution = cx.new(|cx| QueryExecution::start(connection, statement, cx));
        self._current_subscription =
            Some(cx.subscribe(&execution, |this, execution, event, cx| {
                let succeeded = match event {
                    QueryExecutionEvent::Finished => true,
                    QueryExecutionEvent::Failed | QueryExecutionEvent::Cancelled => false,
                    QueryExecutionEvent::RowsLoaded => return,
                };
                if this.current() != Some(execution) {
                    return;
                }
                this.ended_count += 1;
                this._current_subscription = None;
                if succeeded {
                    this.start_next(cx);
                } else {
                    this.pending.clear();
                    this.finish(cx);
                }
                cx.notify();
            }));
        self.executions
            .push(execution);
        cx.notify();
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        if self.duration.is_some() {
            return;
        }
        self.duration = Some(self.started_at.elapsed());
        cx.notify();
    }
}
