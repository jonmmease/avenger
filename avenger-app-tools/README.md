# avenger-app-tools

Reusable interactions for Avenger apps, installed with `AppBuilder::tool` from `avenger-app`. Each
tool registers event streams and has private state, which the app stores and passes only to that
tool. Tools can also declare scenes of their own.

- `tooltips::Tooltips` displays data from marks in lower scenes.
- `text_selection::TextSelection` selects and copies the text of labels in lower scenes.
- `navigation::PanZoom` pans and zooms continuous scales with the `scales` feature.
- `navigation::MapPanZoom` pans and zooms map projections with the `geo` feature.

## Tooltips

```rust
use avenger_app_tools::tooltips::{InstanceTarget, TooltipAnchor, TooltipField, Tooltips};

let tooltips = Tooltips::new("tooltip", 100).over([
    InstanceTarget::nearest(10.0)
        .with_marks(["quakes"])
        .with_header(
            TooltipField::new("magnitude")
                .with_value_format(".1f")
                .with_value_label("Magnitude #value"),
        )
        .with_fields([
            TooltipField::new("depth_km")
                .with_title("Depth")
                .with_value_format(".0f")
                .with_value_label("#value km"),
            TooltipField::new("time").with_title("Date").with_value_format("%b %d, %Y"),
        ]),
    InstanceTarget::exact()
        .with_marks(["countries"])
        .with_anchor(TooltipAnchor::Pointer)
        .with_header("name")
        .with_fields(["continent"]),
]);
```

`Tooltips` works with any app state. The example declares a scene named `"tooltip"` at z-order 100,
above the scenes it describes. The tool stores the pointer position in its private `TooltipState`
and updates when requested or when a lower scene changes. Its marks are noninteractive, so the
tooltip does not block input to lower scenes.

### Targets

An `InstanceTarget` searches lower scenes from top to bottom for one instance with a data row:

- `exact()` finds the topmost instance under the pointer.
- `nearest(max_radius)` finds the instance with the nearest drawn edge within the radius.
  `with_metric` can restrict distance measurement to the x or y direction.
- `with_marks(names)` restricts the search to named marks.

Targets are tried in order. The first match supplies the tooltip. Searches skip stale trees, such as
those retained during navigation, because their geometry no longer matches the displayed marks.

### Fields

A tooltip displays an optional bold header with a color swatch, followed by one row per field. Each
row places the field title on the left and its value on the right. `without_swatch()` hides the
swatch. If no fields are configured, the tooltip shows every data column using its name as the
title.

`TooltipField` uses the same formatting as axis ticks. `value_format` selects a pattern for the text
engine's number or datetime provider, like `tick_format`. `value_label` supplies Typst markup with
`#value` bound to the formatted value, like `tick_label`. Null values display as empty text. A
missing column hides the tooltip and logs an error.

### Placement

- `TooltipAnchor::Data`, the default, places the box beside the instance's drawn edge with a
  callout. It tries right, left, top, then bottom, choosing the first position that fits in the
  window.
- `TooltipAnchor::Pointer` places the box below and to the right of the pointer. It flips
  horizontally or vertically to stay within the window. This suits marks such as map regions that
  have no single anchor point.

The tooltip outlines the instance's fill using the mark's clip. `TooltipStyle` configures the font,
colors, spacing, corners, callout, pointer offset, and outline.

## Navigation

The app owns the scales or projection and uses them to draw its scene. Navigation tools receive
selectors for that state and the interaction rectangle:

```rust
let chart = AvengerApp::builder(state, Chart, engine)
    .tool(PanZoom::xy(PLOT, |s: &ChartState| s.plot, |s: &mut ChartState| &mut s.x, |s| &mut s.y));

let globe = MapPanZoom::new(MAPS, |s: &MapState| s.area, |s: &mut MapState| s.globe.as_mut(), PanMode::Sphere)
    .with_home(|s| s.fitted())
    .with_scale_limits(|s, projection| s.limits(projection));
let map = AvengerApp::builder(state, Map, engine).tool(globe);
```

- A left-button drag pans from the view at the press, keeping the same location under the pointer.
  The wheel zooms around the pointer by a factor of 1.2 per notch, capped at two notches per event.
  One line or 100 pixels of scroll movement counts as a notch. Pan and zoom updates request the
  scene without its tree.
- Each gesture tracks its pending changes and requests a tree rebuild when it settles. Drags settle
  on pause, release, or cancellation. Wheel zoom settles when scrolling stops. The default pause
  delay is 100 ms. Pointer movement alone does not settle wheel zoom. Apps can use
  `Scenes::requested_tree` to request resampling after settling, as the M4 and point-density
  examples do.
- A double-click restores the view saved at the first gesture. `MapPanZoom::with_home` can supply
  another home projection, such as one fitted to the map rectangle.
- `PanZoom` uses `ConfiguredScale::pan` and `zoom`, which linear scales support. Each range must
  span the interaction rectangle. The tool uses only the range direction and maps pointer movement
  to a fraction of the rectangle.
- `MapPanZoom` uses `Projection::pan` and `zoom` in its `PanMode`, with coordinates relative to the
  rectangle's top-left corner. `with_scale_limits` bounds wheel zoom.
- Drags start only inside the rectangle. Wheel events and double-clicks apply only while the pointer
  is inside. Tools with separate areas can share a scene. Navigation tools do not consume events.
- Each tool requests an open-hand cursor over its rectangle and a closed hand during a drag.

## Text selection

```rust
let app = AvengerApp::builder(state, Chart, engine)
    .tool(TextSelection::new("selection", 1000))
    .build()
    .await?;
```

`TextSelection` selects label text from lower scenes without chart-specific configuration.
Text marks can be selected even when they are not interactive. Give the selection scene a z-order
above every scene whose text should be selectable.

- A left-button press starts a selection on the topmost label under the pointer, unless an
  interactive mark covers it in the same scene or a higher scene. The tool consumes the press,
  preventing lower scenes from starting a pan.
- Dragging selects by position after one press, by word after two consecutive presses, and by line
  after three or more. Selection stays within the starting scene and follows label drawing order,
  including intermediate labels in full. Off text, the endpoint moves to the nearest label whose
  nearest point is visible through its clip.
- Copied text joins labels with a tab when the next label continues the last selected line, or a
  newline otherwise. This allows text to paste into spreadsheet cells. Equations copy in full as
  Typst markup.
- A left-button press elsewhere or Escape clears the selection. The selection also clears when
  the anchor label disappears or its source changes, and keeps its text when the label only moves,
  as on a resize.
- A scene whose hit-testing tree is stale, as during a pan, selects none of its text until the
  tree rebuilds, because a stale tree can't show which marks cover a label.
- `with_highlight` sets the highlight color. The tool requests a text cursor over selectable text
  and during a drag, and provides the selected text to the host's copy shortcut. The cursor is
  `CursorStyle::VerticalText` for a label turned within 45 degrees of vertical, and a drag keeps
  its anchor label's cursor.
