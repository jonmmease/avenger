//! A table of rows that a source reads in order, a page at a time as the table scrolls, so a
//! table of millions of rows shows its first rows at once and holds only the rows that it has
//! shown, and a page ahead. A source's rows come from one run of its query, read forward: a query
//! need not give its rows in the same order each time it runs, so pages of separate runs could
//! repeat rows and miss others.
//!
//! The rows read stay, so scrolling back shows them again without reading, and the count of the
//! rows runs beside them, so the scroll bar knows the table's length before the reader gets there.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use avenger_chart::results::QueryResult;
use eframe::egui;
use egui_table::{CellInfo, Column, HeaderCellInfo, HeaderRow, PrefetchInfo, Table, TableDelegate};
use futures::future::LocalBoxFuture;
use futures::stream::{LocalBoxStream, StreamExt};

use crate::exec::{Exec, Task};
use crate::table::{header_ui, padded, stripe, text_ui, TEXT_ROW_HEIGHT};

/// The rows that a table reads ahead of the last row that it shows.
pub const PAGE_ROWS: usize = 256;

/// The rows of a table, which a [`PagedTable`] reads: the schema, the count of the rows, which
/// runs beside the reading, and one stream of the rows in their order. The futures run on a
/// thread of their own, so they need not be `Send`.
pub trait RowSource: Send + Sync {
    fn schema(&self) -> SchemaRef;
    fn count(&self) -> LocalBoxFuture<'static, Result<usize, String>>;
    fn stream(&self) -> RowStream;
}

/// The stream of a source's rows, or the error of a query that doesn't start.
pub type RowStream =
    LocalBoxFuture<'static, Result<LocalBoxStream<'static, Result<RecordBatch, String>>, String>>;

/// What the reader and the count send the table.
enum Event {
    Batch(RecordBatch),
    /// The stream ended, after the rows it read, or with an error.
    End(Result<(), String>),
    Count(Result<usize, String>),
}

/// The count of a table's rows.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Count {
    Counting,
    Known(usize),
    Failed(String),
}

/// A table of a [`RowSource`], with the rows that it has read and the count of all of them.
pub struct PagedTable {
    source: Arc<dyn RowSource>,
    schema: SchemaRef,
    batches: Vec<RecordBatch>,
    /// The rows read, as a result, whose text the table shows as a collected result's.
    rows: QueryResult,
    count: Count,
    /// Whether the stream ended, after its last row or with an error.
    end: Option<Result<(), String>>,
    /// The rows that the table wants read, which the reader reads up to.
    wanted: Arc<AtomicUsize>,
    /// The signal that wakes the reader when the table wants more rows.
    more: Arc<tokio::sync::Notify>,
    events: (Sender<Event>, Receiver<Event>),
    /// The reader and the count, once the table first shows.
    tasks: Vec<Task>,
    /// An error of the reader or the count for the host, which the table gives once.
    error: Option<String>,
}

impl PagedTable {
    /// The table of `source`, which reads nothing until it first shows.
    pub fn new(source: Arc<dyn RowSource>) -> Self {
        let schema = source.schema();
        Self {
            rows: QueryResult::new(schema.clone(), Vec::new()).expect("a result of no rows"),
            source,
            schema,
            batches: Vec::new(),
            count: Count::Counting,
            end: None,
            wanted: Arc::new(AtomicUsize::new(PAGE_ROWS)),
            more: Arc::new(tokio::sync::Notify::new()),
            events: channel(),
            tasks: Vec::new(),
            error: None,
        }
    }

    /// The rows read so far.
    pub fn loaded(&self) -> usize {
        self.rows.num_rows()
    }

    /// The rows that the table has, as far as it knows: its count, or the rows read when the
    /// stream ended first, or else the rows read and a page more, which show as they read.
    pub fn rows(&self) -> usize {
        match (&self.count, &self.end) {
            (_, Some(_)) => self.loaded(),
            (Count::Known(count), None) => *count,
            (_, None) => self.loaded() + PAGE_ROWS,
        }
    }

    /// What the table knows of its rows, for a line over it: their count, or the rows read
    /// while the count runs.
    pub fn status(&self) -> String {
        let rows = |count: usize| match count {
            1 => "1 row".to_string(),
            count => format!("{} rows", grouped(count)),
        };
        match (&self.count, &self.end) {
            (_, Some(Err(_))) => format!("{} read before an error", rows(self.loaded())),
            (Count::Known(count), _) => rows(*count),
            (_, Some(Ok(()))) => rows(self.loaded()),
            (Count::Counting, None) => format!("{} read, counting…", rows(self.loaded())),
            (Count::Failed(_), None) => format!("{} read", rows(self.loaded())),
        }
    }

    /// Starts the reader and the count on `exec`, which wake `ctx` as their results come.
    fn start(&mut self, ctx: &egui::Context, exec: &Exec) {
        let (source, sender, wake) = (self.source.clone(), self.events.0.clone(), ctx.clone());
        let (wanted, more) = (self.wanted.clone(), self.more.clone());
        self.tasks.push(exec.spawn(move || async move {
            let send = |event| {
                let sent = sender.send(event).is_ok();
                wake.request_repaint();
                sent
            };
            let mut stream = match source.stream().await {
                Ok(stream) => stream,
                Err(error) => {
                    send(Event::End(Err(error)));
                    return;
                }
            };
            let mut read = 0;
            loop {
                // On the web, work runs to its end when it starts, so the reader reads every row.
                if read >= wanted.load(Ordering::SeqCst) && cfg!(not(target_arch = "wasm32")) {
                    more.notified().await;
                    continue;
                }
                match stream.next().await {
                    Some(Ok(batch)) => {
                        read += batch.num_rows();
                        if !send(Event::Batch(batch)) {
                            return;
                        }
                    }
                    Some(Err(error)) => {
                        send(Event::End(Err(error)));
                        return;
                    }
                    None => {
                        send(Event::End(Ok(())));
                        return;
                    }
                }
            }
        }));
        let (source, sender, wake) = (self.source.clone(), self.events.0.clone(), ctx.clone());
        self.tasks.push(exec.spawn(move || async move {
            let _ = sender.send(Event::Count(source.count().await));
            wake.request_repaint();
        }));
    }

    /// Takes the reader's rows and the count.
    fn receive(&mut self) {
        let mut read = false;
        while let Ok(event) = self.events.1.try_recv() {
            match event {
                Event::Batch(batch) if batch.num_rows() > 0 => {
                    self.batches.push(batch);
                    read = true;
                }
                Event::Batch(_) => {}
                Event::End(end) => {
                    if let Err(error) = &end {
                        self.error = Some(error.clone());
                    }
                    self.end = Some(end);
                }
                Event::Count(Ok(count)) => self.count = Count::Known(count),
                Event::Count(Err(error)) => {
                    self.error = Some(error.clone());
                    self.count = Count::Failed(error);
                }
            }
        }
        if read {
            self.rows = QueryResult::new(self.schema.clone(), self.batches.clone())
                .expect("a result of rows without charts");
        }
    }

    /// Asks the reader for the rows up to `end`, and a page ahead of them.
    fn want(&self, end: usize) {
        let wanted = end + PAGE_ROWS;
        if self.wanted.fetch_max(wanted, Ordering::SeqCst) < wanted {
            self.more.notify_one();
        }
    }

    /// Shows the table in the rest of `ui`, under a line of its row count, reading on `exec`,
    /// with the id `id`. Returns an error of the reader or the count once, for the host to show.
    pub fn show(&mut self, ui: &mut egui::Ui, exec: &Exec, id: u64) -> Option<String> {
        if self.tasks.is_empty() {
            self.start(ui.ctx(), exec);
        }
        self.receive();
        padded(ui, |ui| {
            ui.label(egui::RichText::new(self.status()).small().weak())
        });
        let columns: Vec<Column> = (self.schema.fields().iter())
            .map(|field| {
                Column::new(100.0)
                    .range(egui::Rangef::new(40.0, 480.0))
                    .resizable(true)
                    .id(egui::Id::new(("paged column", field.name())))
            })
            .collect();
        let mut grid = Grid {
            table: self,
            end: 0,
        };
        // The columns size to their contents once the first rows have read.
        let sized = grid.table.loaded() > 0;
        Table::new()
            .id_salt(("paged table", id, sized))
            .num_rows(grid.table.rows() as u64)
            .columns(columns)
            .headers([HeaderRow::new(40.0)])
            .show(ui, &mut grid);
        let end = grid.end;
        self.want(end);
        self.error.take()
    }
}

impl Drop for PagedTable {
    /// Stops the reader and the count with the table.
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// The delegate of a paged table for one frame, which notes the end of its visible rows.
struct Grid<'a> {
    table: &'a PagedTable,
    end: usize,
}

impl TableDelegate for Grid<'_> {
    fn prepare(&mut self, info: &PrefetchInfo) {
        self.end = info.visible_rows.end as usize;
    }

    fn header_cell_ui(&mut self, ui: &mut egui::Ui, cell: &HeaderCellInfo) {
        if let Some(field) = self.table.schema.fields().get(cell.col_range.start) {
            header_ui(ui, field.name(), &field.data_type().to_string());
        }
    }

    fn cell_ui(&mut self, ui: &mut egui::Ui, cell: &CellInfo) {
        let (row, column) = (cell.row_nr as usize, cell.col_nr);
        stripe(ui, row);
        match row < self.table.loaded() {
            true => text_ui(ui, self.table.rows.value_text(row, column).as_deref()),
            // A row that the reader hasn't read yet.
            false => {
                padded(ui, |ui| ui.label(egui::RichText::new("…").weak()));
            }
        }
    }

    fn default_row_height(&self) -> f32 {
        TEXT_ROW_HEIGHT
    }
}

/// `count` with its digits grouped by commas, as `1,234,567`.
fn grouped(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}
