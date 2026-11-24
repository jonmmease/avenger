//! The panel of a query and its stages, in frames of egui without a window: the shortcut that
//! opens it, the stages that a reader opens and that a new text keeps or closes, a debugger's
//! current stage, and the failing query that colors its button.

use std::sync::Arc;

use arrow::array::Int64Array;
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use avenger_results_egui::{
    Code, CodeOperator, Exec, QueryPanel, RowSource, RowStream, StageEvent, QUERY_SHORTCUT,
};
use eframe::egui;
use futures::future::LocalBoxFuture;
use futures::stream::{self, StreamExt};

/// The code of `text`, whose operators are its `|>`s.
fn code(text: &str) -> Code {
    Code {
        text: text.to_string(),
        operators: (text.match_indices("|>").enumerate())
            .map(|(index, (pipe, _))| CodeOperator {
                ordinal: index + 1,
                pipe,
                name: "|> WHERE".into(),
            })
            .collect(),
        error: None,
    }
}

/// Three rows of one column.
struct Three;

impl RowSource for Three {
    fn schema(&self) -> SchemaRef {
        Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)]))
    }

    fn count(&self) -> LocalBoxFuture<'static, Result<usize, String>> {
        Box::pin(async { Ok(3) })
    }

    fn stream(&self) -> RowStream {
        let schema = self.schema();
        Box::pin(async move {
            let values = Int64Array::from_iter_values(0..3);
            let batch = RecordBatch::try_new(schema, vec![Arc::new(values)]).unwrap();
            Ok(stream::iter([Ok(batch)]).boxed_local())
        })
    }
}

/// Shows `panel` in a frame of an 800 by 600 screen, with `events`.
fn frame(ctx: &egui::Context, panel: &mut QueryPanel, exec: &Exec, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(800.0, 600.0),
        )),
        events,
        ..Default::default()
    };
    let _ = ctx.run(input, |ctx| panel.show(ctx, exec));
}

#[test]
fn the_shortcut_opens_and_closes_the_panel() {
    let (ctx, exec) = (egui::Context::default(), Exec::new().unwrap());
    let mut panel = QueryPanel::new();
    panel.set_code(Some(code("FROM t\n|> WHERE x\n")));
    let press = egui::Event::Key {
        key: QUERY_SHORTCUT.logical_key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: QUERY_SHORTCUT.modifiers,
    };
    frame(&ctx, &mut panel, &exec, vec![press.clone()]);
    assert!(panel.is_open());
    // The panel draws its code and an open stage's table in frames as it opens.
    panel.toggle_stage(1);
    panel.set_stage(1, Ok(Arc::new(Three)));
    for _ in 0..5 {
        frame(&ctx, &mut panel, &exec, Vec::new());
    }
    frame(&ctx, &mut panel, &exec, vec![press]);
    assert!(!panel.is_open());
}

#[test]
fn stages_keep_their_ordinals_as_the_text_changes_and_close_past_its_end() {
    let mut panel = QueryPanel::new();
    panel.set_code(Some(code("FROM t\n|> WHERE x\n|> LIMIT 3\n")));
    panel.toggle_stage(1);
    panel.toggle_stage(2);
    assert_eq!(
        panel.take_events(),
        [StageEvent::Open(1), StageEvent::Open(2)]
    );
    // The same text with an error changes nothing else.
    panel.set_code(Some(Code {
        error: Some(2),
        ..code("FROM t\n|> WHERE x\n|> LIMIT 3\n")
    }));
    assert!(panel.take_events().is_empty());
    assert!(panel.fails());
    // A new text runs the open stages again, and one past its operators closes.
    panel.set_code(Some(code("FROM t\n|> WHERE y\n")));
    assert_eq!(panel.take_events(), [StageEvent::Open(1)]);
    assert_eq!(panel.open_stages(), [1]);
    assert!(!panel.fails());
    // A second click closes a stage.
    panel.toggle_stage(1);
    assert_eq!(panel.take_events(), [StageEvent::Close(1)]);
    assert!(panel.open_stages().is_empty());
}

#[test]
fn a_debugger_s_stop_opens_its_stage_alone_and_a_failed_stage_fails_the_query() {
    let mut panel = QueryPanel::new();
    panel.set_code(Some(code("FROM t\n|> WHERE x\n|> LIMIT 3\n")));
    panel.toggle_stage(1);
    panel.take_events();
    panel.set_current(Some(2));
    assert!(panel.is_open());
    assert_eq!(panel.open_stages(), [2]);
    assert!(panel.take_events().is_empty());
    panel.set_stage(2, Err("Unrecognized name: nope".into()));
    assert!(panel.fails());
    // A stage that isn't open takes no rows.
    panel.set_stage(1, Ok(Arc::new(Three)));
    assert_eq!(panel.open_stages(), [2]);
    // At a statement's result, every stage closes.
    panel.set_current(None);
    assert!(panel.open_stages().is_empty());
}
