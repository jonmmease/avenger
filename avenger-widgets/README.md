# avenger-widgets

Controls for an `AvengerApp`: buttons, check boxes, radio groups, sliders, text inputs and group
boxes. A `Widgets` tool draws them in its own scene and routes pointer, key and paste events to
them. Other crates can add controls through the same `Widget` trait that the built-ins implement.

Value controls read app state through `value` and update it through `on_change`. Buttons use
`on_press`, and group boxes frame other controls. Callbacks return the scenes to rebuild.
Widgets keep their interaction state, such as a drag's starting value. The app places them,
so controls and plots share one `avenger-layout` tree.

## Add widgets to an app

Build the widgets with a `place` closure that reads their rectangles from app state. Keep a
clone for layout measurements. Install the original as a tool:

```rust
let widgets = Widgets::new("controls", 20, |s: &State, id| s.controls.get(id).copied())
    .with(
        Slider::new("opacity", 0.0..=1.0)
            .label("Opacity")
            .value(|s: &State| s.opacity)
            .on_change(|s, opacity| {
                s.opacity = opacity;
                Rebuild::scene("chart")
            })
            .readout(".0%"),
    )
    .with(
        Checkbox::new("legend")
            .label("Show legend")
            .value(|s: &State| s.show_legend)
            .on_change(|s, show| {
                s.show_legend = show;
                Rebuild::tree("chart")
            }),
    );

let app = AvengerApp::builder(state, Chart { widgets: widgets.clone() }, engine)
    .tool(widgets)
    .build()
    .await?;
```

The app's build lays out the widgets with the rest of the scene. Give each widget a layout leaf.
Forward its measurement requests to `widgets.measure(engine)`. Store its solved rectangle in
app state for the `place` closure:

```rust
impl Measure<Id> for ChartMeasure<'_> {
    fn height(&mut self, id: &Id, width: f32) -> Option<Measurement> {
        match id {
            Id::Widget(name) => self.widgets.height(name, width),
            _ => None,
        }
    }
    // `width` and `max_content` forward the same way.
}
```

A widget without a rectangle is hidden and cannot receive input or gain focus. Widgets draw in
insertion order, so later widgets receive pointer events where their rectangles overlap. Tab
visits widgets in insertion order. The `iris` example places a column of controls beside its
chart.

## How the tool handles input

The tool routes each event to at most one widget:

- A pointer event goes to the widget that holds the current press, or else to the topmost widget
  under the pointer. A press that begins on a widget belongs to it until the release, so a slider
  drag continues outside the slider. During a press that began elsewhere, such as a pan, no
  widget is hovered.
- A key, paste or input method event goes to the widget with keyboard focus. Tab and Shift+Tab
  visit the placed, enabled, focusable widgets. Moving past either end clears focus and passes
  Tab to the app. Pressing a focusable widget focuses it. Pressing outside every widget clears
  focus, including a press that a scene above takes, such as one that starts a text selection.
  A text input draws its focus ring whenever it has focus, and the other widgets only when focus
  comes from the keyboard. Tab traversal is limited to one `Widgets` tool, so keep the app's
  widgets in a single tool.
- The focused widget's copy text, such as a text input's selection, becomes the app's copy
  text, which the host copies on Cmd+C or Ctrl+C. A focused text input owns copy and cut, even
  without a selection, over other tools' selections, such as a text selection scene's.
- Window focus loss sends `Cancel` to the focused widget to clear held-key state. The widget
  keeps its keyboard focus.

Widgets consume pointer events by default, preventing scenes below them from handling those
events. Built-in controls ignore keys they do not use, so app shortcuts remain available.
The tool reuses a widget's drawing while its view, rectangle and look are unchanged. A chart
pan can therefore retain the controls scene. A build runs only when some scene is requested,
so an external handler that changes a value that only a widget shows requests the `Widgets`
scene.

## Lay out responsive widgets

A widget's measurements depend on its configuration, style and available space. They do not
depend on app state, so a value change does not alter its measured size. A radio group with
automatic direction uses a row when it fits the available width and a column otherwise.

A group box draws a border around controls, with its title in a gap in the top edge.
`widgets.group_box_layout` builds a container from the group box's id, a container id, the label
engine and the child nodes. Use the container's region as the group box's rectangle. Add the
group box before the controls inside it so those controls receive pointer events first:

```rust
let points = widgets
    .group_box_layout(
        "points",
        Id::Widget("points".into()),
        engine,
        [widget("opacity"), widget("size")],
    )
    .gap(Gap::new(10.0));
```

## Built-in widgets

| Widget | Pointer | Keys |
| --- | --- | --- |
| `Button` | Activates on a left-button release inside it. | Enter activates on press, and Space on release. Escape cancels a pending press. A held key activates once. |
| `Checkbox` | Toggles on a left-button release inside it. | Space toggles on release. Escape cancels a pending press. |
| `RadioGroup` | Selects an item when a press begins and ends on it. | Arrow keys move the selection and wrap at either end. |
| `Slider` | Pressing or dragging on the track sets the value at the pointer. | Arrow keys move one step, Page Up and Page Down move ten steps, and Home and End select the endpoints. |
| `TextInput` | A press places the caret. A drag or a press with Shift selects, a double press selects a word, and a triple press selects all. A drag after a double press selects whole words. | Typing, editing and clipboard keys, as [Edit text](#edit-text) lists. |
| `GroupBox` | Consumes presses inside it that miss its widgets. | None. It isn't a Tab stop. |

Buttons, check boxes, sliders, radio groups and text inputs have optional labels, set with
`label`.
Omitted labels reserve no space. A group box takes its title in `new`. Each built-in widget
also accepts an optional `enabled` closure. A disabled widget keeps its place, draws dimmed,
receives no input and is not a Tab stop. Disabling a group box does not disable the controls
inside it.

A slider calls `on_change` whenever it assigns a different value, including when it restores a
canceled drag. It calls `on_settle` once a gesture that changed the value ends, passing the final
value. Escape restores the drag's starting value immediately, but settlement waits for release
or cancellation by the tool. Keyboard changes settle on key release, widget blur or cancellation.
When `on_change` defers a chart's hit-tree rebuild with `Rebuild::scene`, use `on_settle` to
request `Rebuild::tree`, including after cancellation.

With `step`, new slider values snap to the range's start, a step after it or its end. Canceling
a drag restores the exact starting value, even if it is off-step. The optional readout uses the
label engine's number provider, like an axis's `tick_format`. A pattern without a precision takes
the precision of the slider's step, or of one hundredth of a continuous slider's range, as an
axis's ticks take the precision of their step. `readout_label` wraps the formatted value in
Typst markup, like `tick_label`. The readout reserves the larger of its widths at the range's
endpoints.

## Edit text

A text input edits one line of plain text in app state. It reads the value through `value` and
passes each edit to `on_change`. `on_submit` runs on Enter, and without it, Enter reaches the
app. A `placeholder` shows in the box while the value is empty. The text scrolls sideways to
keep the caret in the box. Keyboard focus selects all of the text.

Typed text replaces the selection. These keys move the caret and edit the text:

| Keys | Action |
| --- | --- |
| Left, Right | Move one character. With Alt or Ctrl, move to a word's start or end. With Cmd, move to the text's start or end. |
| Home, End | Move to the text's start or end. |
| Backspace, Delete | Delete the selection, or as far back or ahead as Left or Right with the same modifiers moves. |
| Shift with a movement | Extend the selection. |
| Cmd+A or Ctrl+A | Select all of the text. |
| Cmd+X or Ctrl+X | Cut the selection. |
| Enter | Run `on_submit`. |

The host copies the selection on Cmd+C or Ctrl+C, and on Cmd+X or Ctrl+X before the input
deletes it. The host also delivers pasted text as a paste event. It reads the clipboard on
Cmd+V or Ctrl+V on native targets, and takes the page's paste event in the browser. A paste
replaces each line break or tab with a space.

Typed text comes from the text that each key press types, and from the text that an input
method composes, such as é from Option+E and E on macOS. While a text input has focus, the host
lets input methods compose text and places their candidate windows beside its caret. Text that
an input method is still composing, such as the accent after Option+E, shows underlined in place
of the selection, and the input inserts the text that the input method commits. The input has
no undo, so Cmd+Z and Ctrl+Z reach the app, as Escape, Up and Down do. The caret does not
blink.

## Write a widget

Implement `Widget<State>`. `WidgetContext` provides the label engine and style to both custom
and built-in widgets.

- `view` collects everything that `draw` shows from app state and the widget's own state. The
  tool compares views to decide what to draw again, so `draw` must read app state only through
  the view.
- `max_content`, `height` and `width` provide measurements for `avenger_layout::Measure` and
  must not depend on app state.
- `event` receives pointer and key events as `WidgetEvent::Input`, plus `Focus`, `Blur` and
  `Cancel` notifications. If a pressed or focused widget loses its rectangle or becomes
  disabled, the tool sends the corresponding `Cancel` or `Blur` on the next event.
- `disposition` decides whether an input event is ignored, handled or consumed.
- `copy_text` returns the text in the latest drawing's view that the copy shortcut copies while
  the widget has focus.
- `text_entry` returns the caret's area for a widget that takes typed text, so that while the
  widget has focus, the host lets input methods compose text and places their candidate
  windows beside the caret.
- When a `TextSelection` scene is above the widgets, a label is selectable unless an
  interactive mark drawn after it covers it. Built-in widgets cover the labels that a press acts
  on, such as a button's and a check box's, and leave titles, such as a slider's label and its
  readout, selectable.
- `draw` can fail, as on invalid markup. The tool draws the other widgets and reports the first
  failure from its update.

## Style

`WidgetStyle` sets the font, colors, sizes and spacing. `WidgetStyle::default()` is a light
style, and `WidgetStyle::dark()` is a dark style. Each palette has a `Paint` for the normal,
hovered, pressed and disabled states, selected by `palette.paint(look)`. The presets dim disabled
widgets by blending their normal colors toward the background while preserving alpha. Opaque
parts stay opaque, so overlapping parts do not show through each other.
