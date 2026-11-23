//! A paged table, in frames of egui without a window: it reads the rows that it shows and a page
//! ahead from one stream, counts the rest beside them, and reads more as it scrolls.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use arrow::array::Int64Array;
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use avenger_results_egui::{Exec, PagedTable, RowSource, RowStream, PAGE_ROWS};
use eframe::egui;
use futures::future::LocalBoxFuture;
use futures::stream::{self, StreamExt};

/// The rows of the source, and of each of its batches.
const ROWS: usize = 100_000;
const BATCH: usize = 64;

/// A source of the integers from 0, in batches, which counts the batches that its stream gives.
struct Numbers {
    schema: SchemaRef,
    given: Arc<AtomicUsize>,
}

impl RowSource for Numbers {
    fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    fn count(&self) -> LocalBoxFuture<'static, Result<usize, String>> {
        Box::pin(async { Ok(ROWS) })
    }

    fn stream(&self) -> RowStream {
        let (schema, given) = (self.schema.clone(), self.given.clone());
        Box::pin(async move {
            let batches = (0..ROWS / BATCH).map(move |index| {
                given.fetch_add(1, Ordering::SeqCst);
                let start = (index * BATCH) as i64;
                let values = Int64Array::from_iter_values(start..start + BATCH as i64);
                Ok(RecordBatch::try_new(schema.clone(), vec![Arc::new(values)]).unwrap())
            });
            Ok(stream::iter(batches).boxed_local())
        })
    }
}

/// Shows `table` in frames of an 800 by 600 screen until its reader and its count rest, after a
/// turn of the mouse wheel over it of `scroll` points down.
fn settle(ctx: &egui::Context, table: &mut PagedTable, exec: &Exec, scroll: f32) {
    let mut last = usize::MAX;
    for frame in 0..200 {
        let mut events = vec![egui::Event::PointerMoved(egui::pos2(400.0, 300.0))];
        if frame == 0 && scroll > 0.0 {
            events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -scroll),
                modifiers: egui::Modifiers::NONE,
            });
        }
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                table.show(ui, exec, 1);
            });
        });
        let counting = table.status().ends_with("counting…");
        if frame > 2 && table.loaded() == last && !counting {
            return;
        }
        last = table.loaded();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_paged_table_reads_what_it_shows_and_a_page_ahead() {
    let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]));
    let given = Arc::new(AtomicUsize::new(0));
    let source = Numbers {
        schema,
        given: given.clone(),
    };
    let mut table = PagedTable::new(Arc::new(source));
    let (ctx, exec) = (egui::Context::default(), Exec::new().unwrap());
    settle(&ctx, &mut table, &exec, 0.0);
    // The first screen of rows and a page ahead read, and the count knows the rest.
    let loaded = table.loaded();
    assert!((PAGE_ROWS..2 * PAGE_ROWS).contains(&loaded), "{loaded}");
    assert_eq!(given.load(Ordering::SeqCst), loaded / BATCH);
    assert_eq!(table.rows(), ROWS);
    assert_eq!(table.status(), "100,000 rows");
    // Scrolling reads on, as far as the rows that show and a page ahead.
    settle(&ctx, &mut table, &exec, 24.0 * 1000.0);
    let scrolled = table.loaded();
    assert!(
        (1001..1000 + 2 * PAGE_ROWS).contains(&scrolled),
        "{scrolled}"
    );
}
