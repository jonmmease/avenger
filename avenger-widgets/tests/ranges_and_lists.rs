//! The range slider and the check box list, driven through the `Widget` interface: a drag moves
//! the nearer thumb without passing the other, and a click checks or unchecks an item.

use avenger_eventstream::scene::{
    ModifiersState, SceneCursorMovedEvent, SceneGraphEvent, SceneMouseDownEvent, SceneMouseUpEvent,
};
use avenger_eventstream::stream::Rebuild;
use avenger_eventstream::window::MouseButton;
use avenger_layout::Rect;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_typst_label::{bundled_label_engine, LabelEngine};
use avenger_widgets::{
    CheckboxList, Look, Placed, RangeSlider, Widget, WidgetContext, WidgetEvent, WidgetStyle,
};

#[derive(Default)]
struct State {
    range: [f64; 2],
    settled: Vec<[f64; 2]>,
    checked: Vec<&'static str>,
}

fn engine() -> LabelEngine {
    bundled_label_engine().with_number_formatting(std::sync::Arc::new(
        avenger_format_number_d3::D3NumberFormatProvider::new(),
    ))
}

fn down(position: [f32; 2]) -> SceneGraphEvent {
    SceneGraphEvent::MouseDown(SceneMouseDownEvent {
        position,
        button: MouseButton::Left,
        mark_instance: None,
        modifiers: ModifiersState::default(),
        count: 1,
    })
}

fn moved(position: [f32; 2]) -> SceneGraphEvent {
    SceneGraphEvent::CursorMoved(SceneCursorMovedEvent {
        position,
        mark_instance: None,
        modifiers: ModifiersState::default(),
    })
}

fn up(position: [f32; 2]) -> SceneGraphEvent {
    SceneGraphEvent::MouseUp(SceneMouseUpEvent {
        position,
        button: MouseButton::Left,
        mark_instance: None,
        modifiers: ModifiersState::default(),
    })
}

fn placed(rect: Rect) -> Placed {
    Placed {
        rect,
        look: Look {
            enabled: true,
            ..Look::default()
        },
    }
}

/// Sends `events` to `widget` placed at `rect`.
fn send<W: Widget<State>>(
    widget: &W,
    rect: Rect,
    state: &mut State,
    own: &mut W::State,
    events: &[SceneGraphEvent],
) {
    let engine = engine();
    let style = WidgetStyle::default();
    let cx = WidgetContext {
        engine: &engine,
        style: &style,
    };
    for event in events {
        let _: Rebuild = widget.event(WidgetEvent::Input(event), &placed(rect), &cx, state, own);
    }
}

fn range_slider() -> RangeSlider<State> {
    RangeSlider::new("magnitude", 0.0..=10.0)
        .step(1.0)
        .value(|state: &State| state.range)
        .on_change(|state, range| {
            state.range = range;
            Rebuild::none()
        })
        .on_settle(|state, range| {
            state.settled.push(range);
            Rebuild::none()
        })
}

#[test]
fn a_drag_moves_the_nearer_thumb_without_passing_the_other() {
    let slider = range_slider();
    let rect = Rect::new(0.0, 0.0, 220.0, 30.0);
    let mut state = State {
        range: [2.0, 8.0],
        ..State::default()
    };
    let mut own = Default::default();
    let style = WidgetStyle::default();
    // The track runs from half a thumb in to half a thumb from the end.
    let half = style.thumb_size / 2.0;
    let x = |value: f32| half + value / 10.0 * (220.0 - 2.0 * half);
    // The middle of the row of the track.
    let y = half;
    // A press near the low thumb moves it, and a drag past the high thumb stops at it.
    send(
        &slider,
        rect,
        &mut state,
        &mut own,
        &[down([x(3.0), y]), moved([x(9.0), y]), up([x(9.0), y])],
    );
    assert_eq!(state.range, [8.0, 8.0]);
    assert_eq!(state.settled, [[8.0, 8.0]]);
    // A press near the high end moves the high thumb.
    send(
        &slider,
        rect,
        &mut state,
        &mut own,
        &[down([x(10.0), y]), up([x(10.0), y])],
    );
    assert_eq!(state.range, [8.0, 10.0]);
}

#[test]
fn a_range_slider_draws_two_thumbs_and_its_readout() {
    let slider = range_slider().readout("d");
    let state = State {
        range: [2.0, 8.0],
        ..State::default()
    };
    let engine = engine();
    let style = WidgetStyle::default();
    let cx = WidgetContext {
        engine: &engine,
        style: &style,
    };
    let view = slider.view(&state, &Default::default());
    let marks = slider
        .draw(&view, &placed(Rect::new(0.0, 0.0, 260.0, 30.0)), &cx)
        .unwrap();
    let thumbs = (marks.iter())
        .filter(|mark| matches!(mark, SceneMark::Rect(rect) if rect.name == "thumb"))
        .count();
    assert_eq!(thumbs, 2);
    let readout = marks.iter().find_map(|mark| match mark {
        SceneMark::Text(text) if text.name == "readout" => {
            Some(text.text.as_vec(1, None)[0].clone())
        }
        _ => None,
    });
    assert!(readout.is_some_and(|text| text.contains('2') && text.contains('8')));
}

#[test]
fn a_click_checks_and_unchecks_an_item_of_a_list() {
    let list = CheckboxList::new(
        "origin",
        [("USA", "USA"), ("Europe", "Europe"), ("Japan", "Japan")],
    )
    .direction(avenger_widgets::RadioDirection::Column)
    .value(|state: &State| state.checked.clone())
    .on_change(|state, checked| {
        state.checked = checked;
        Rebuild::none()
    });
    let mut state = State {
        checked: vec!["USA", "Japan"],
        ..State::default()
    };
    let mut own = Default::default();
    let rect = Rect::new(0.0, 0.0, 120.0, 90.0);
    let engine = engine();
    let style = WidgetStyle::default();
    let cx = WidgetContext {
        engine: &engine,
        style: &style,
    };
    let view = list.view(&state, &own);
    let marks = list.draw(&view, &placed(rect), &cx).unwrap();
    let boxes: Vec<[f32; 2]> = (marks.iter())
        .filter_map(|mark| match mark {
            SceneMark::Rect(rect) if rect.name == "checkbox" => {
                let x = rect.x.as_vec(1, None)[0];
                let y = rect.y.as_vec(1, None)[0];
                Some([x + 2.0, y + 2.0])
            }
            _ => None,
        })
        .collect();
    assert_eq!(boxes.len(), 3);
    // A click on Europe checks it, in the order of the items, and a click on USA unchecks it.
    send(
        &list,
        rect,
        &mut state,
        &mut own,
        &[down(boxes[1]), up(boxes[1])],
    );
    assert_eq!(state.checked, ["USA", "Europe", "Japan"]);
    send(
        &list,
        rect,
        &mut state,
        &mut own,
        &[down(boxes[0]), up(boxes[0])],
    );
    assert_eq!(state.checked, ["Europe", "Japan"]);
}
