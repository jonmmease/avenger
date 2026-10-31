# avenger-app-tools

Reusable interactions for Avenger apps, installed with `AppBuilder::tool` from `avenger-app`. Each
tool registers event streams and has private state, which the app stores and passes only to that
tool. Tools can also declare scenes of their own.

- `tooltips::Tooltips` displays data from marks in lower scenes.
- `text_selection::TextSelection` selects and copies the text of labels in lower scenes.
- `navigation::PanZoom` pans and zooms continuous scales with the `scales` feature.
- `navigation::MapPanZoom` pans and zooms map projections with the `geo` feature.
- `brushes::Brushes` selects ranges of continuous scales, and of longitude and latitude on maps,
  with boxes, and instances with clicks and lassos, with the `scales` or `geo` feature.

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
tooltip does not block input to lower scenes. To turn tooltips on and off, such as with a check
box, register the tool with `AppBuilder::tool_when`.

### Targets

An `InstanceTarget` searches lower scenes from top to bottom for one instance with a data row:

- `exact()` finds the topmost instance under the pointer.
- `nearest(max_radius)` finds the instance with the nearest drawn edge within the radius.
  `with_metric` can restrict distance measurement to the x or y direction.
- `with_marks(names)` restricts the search to named marks.

Targets are tried in order. The first match supplies the tooltip. Searches skip stale trees, such as
those retained during navigation, because their geometry no longer matches the displayed marks.
`Tooltips::targets` reads the targets from the app state on each update instead of a fixed list,
for an app whose marks change, such as an app that swaps its charts.

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

let facets = KeyedPanZoom::x(CHART, |s: &FacetState| s.panels(), |s: &mut FacetState, panel| s.x_of(panel))
    .on_settle(|s, panel| s.reload(panel))
    .bounds(|s, panel| [Some(s.extent(panel)), None])
    .min_span([Some(10.0), None]);
```

- A left-button drag pans from the view at the press, keeping the same location under the pointer,
  once the pointer moves more than 4 pixels from the press, so a click doesn't pan. The wheel
  zooms around the pointer by a factor of 1.2 per notch, capped at two notches per event. One line
  or 100 pixels of scroll movement counts as a notch. Pan and zoom updates request the scene
  without its tree.
- Each gesture tracks its pending changes and requests a tree rebuild when it settles. Drags settle
  on pause, release, or cancellation. Wheel zoom settles when scrolling stops. The default pause
  delay is 100 ms. Pointer movement alone does not settle wheel zoom. Apps can use
  `Scenes::requested_tree` to request resampling after settling, as the M4 and point-density
  examples do.
- A double-click restores the view saved at the first gesture. `PanZoom::with_home` and
  `MapPanZoom::with_home` can supply another home view, such as the scales that the app first drew
  or a projection fitted to the map rectangle.
- `PanZoom` uses `ConfiguredScale::pan` and `zoom`, which linear and time scales support. Each range
  must span the interaction rectangle. The tool uses only the range direction and maps pointer
  movement to a fraction of the rectangle.
- `KeyedPanZoom` moves the scales of each of a set of areas, such as the panels of a facet grid,
  which the app names with keys and reads from its state on each event. A press, a wheel event, or a
  double-click acts on the first area that contains the pointer, and a drag keeps the key of its
  press. Panels that share a scale return the same scale, so a pan in one moves them all.
  `PanZoom` is the keyed tool with one area.
- `on_settle` runs when a gesture that changed a view settles, and after a double-click, so an app
  can load data for the new view. `bounds` keeps each domain inside `[low, high]`, and `min_span`
  limits zooming in, both in the values that `ConfiguredScale::invert_scalar` gives, which are epoch
  milliseconds for a time scale.
- `MapPanZoom` uses `Projection::pan` and `zoom` in its `PanMode`, with coordinates relative to the
  rectangle's top-left corner. `with_scale_limits` bounds wheel zoom, and `on_settle` runs as it
  does for scales, so an app can load data for the new projection. `KeyedMapPanZoom` moves the
  projections of keyed areas, each in the `PanMode` of its key, as `KeyedPanZoom` moves scales.
  `MapPanZoom` is the keyed tool with one area.
- Drags start only inside the rectangle. Wheel events and double-clicks apply only while the pointer
  is inside. Tools with separate areas can share a scene. Navigation tools do not consume events.
- Each tool requests an open-hand cursor over its rectangle and a closed hand during a drag.

## Brushes

```rust
let brushes = Brushes::new("brushes", 10).with(
    IntervalBrush::xy("select")
        .area(|s: &ChartState| s.plot)
        .x_scale(|s| s.x.clone())
        .y_scale(|s| s.y.clone())
        .value(|s| s.brush)
        .on_change(|s, interval| {
            s.brush = interval;
            Rebuild::scene(CHART)
        })
        .on_settle(|_, _| Rebuild::tree(CHART))
        .active(|s| s.mode == Mode::Select),
);
```

One `Brushes` tool draws every brush of the app in its own scene, so a grid of panels costs one
render pass. Give the scene a z-order above the chart scenes and below tooltips. The brushes are:

- `IntervalBrush::x`, `y`, and `xy`, which select ranges of continuous scales with a box and report
  an `Interval` in the scales' domains. The scales must invert, as linear, log, pow, and symlog
  scales do.
- `PointBrush`, which selects instances by clicking them anywhere in the app. A click replaces the
  selection, and a Shift+click adds or removes one instance.
- `LassoBrush`, which draws freehand paths and reports a `Lasso`: the paths' polygons and the keys
  of the instances whose centers lie inside them. A Shift+drag adds a path.
- `GeoBoxBrush::lon_lat`, `lat`, and `lon`, with the `geo` feature, which select ranges of
  longitude and latitude through a map projection and report a `GeoBox`.

Each box and lasso brush covers one area. `KeyedIntervalBrush`, `KeyedLassoBrush`, and
`KeyedGeoBoxBrush` cover a set of areas that the app names with keys, as `Keyed areas` describes.
`KeyedPointBrush` gives each area its own instances and its own selection, and acts on the clicks
in its areas, so a click away from every instance clears the selection of its area alone.

The app stores each selection in its own state, in data terms. `value` reads it, `on_change`
receives each next value during a gesture, and `on_settle` receives the last one when the gesture
ends. A point brush has `on_change` only. The brush draws the stored value, so `on_change` can
constrain it. A typical `on_change` requests the chart scene to highlight the selection, and
`on_settle` requests the chart with its tree, as a pan does.

### Claims

The tool applies these rules to a left-button press, considering only brushes whose `active`
predicate holds:

1. A press on a box's edge or corner resizes the box, and a press inside it moves it.
2. Otherwise, the first brush with an area that contains the press draws a new box or path there.
3. Otherwise, the press passes to the scenes below, where a `PanZoom` can pan.

A press whose pointer stays within 4 pixels until its release is a click, which a point brush acts
on, and which clears the box or lasso in its area unless a point brush picks an instance with it.
Escape restores the value at the press. The
second press of a double-click passes to the scenes below, and so does the wheel while no brush
holds a press, so a `PanZoom` still restores its home view and zooms.

### Instances

```rust
let id = |datum: &Datum| {
    let ids = datum.rows.column_by_name("id")?.as_primitive::<UInt32Type>();
    Some(ids.value(datum.row))
};
let picks = PointBrush::new(Instances::marks(["flowers"]).key(id))
    .value(|s: &ChartState| s.picked.clone())
    .on_change(|s, picked| {
        s.picked = picked;
        Rebuild::tree(CHART)
    });
```

Point and lasso brushes select the instances of named marks in the scenes below the brushes, by a
key from each instance's row. Key the instances by a column, such as an ID, because filtered
batches give different marks' rows the same indices. A brush selects nothing while a target tree
is stale, as during a zoom that hasn't settled, so request a scene's tree whenever its marks
settle. A lasso reads the instances' centers at its press, so the app can highlight the selection
during the drag without rebuilding the chart's tree. `instances_from` reads the instances from the
app state at each click or press instead, for an app whose marks change.

### Maps

On a map, `GeoBoxBrush` draws its box through the area's projection: a globe shows a curved
quadrilateral, and a flat map cuts the box at the antimeridian. A `LassoBrush` with a `projection`
in place of scales reports its polygons in longitude and latitude, wound as RFC 7946 winds
GeoJSON, so `avenger_geo::polygon_contains` tests a place against each one. A place lies inside
the lasso when one of its polygons contains it.

### Box zoom

An interval brush zooms as well as selects: the app shows the box while the drag lasts, and on
release sets the scales' domains to it and clears it. Escape and a click settle `None`, which
zooms nothing.

```rust
let zoom = IntervalBrush::xy("zoom")
    .handles(false)
    .area(|s: &ChartState| s.plot)
    .x_scale(|s| s.x.clone())
    .y_scale(|s| s.y.clone())
    .value(|s| s.zoom_box)
    .on_change(|s, interval| {
        s.zoom_box = interval;
        Rebuild::none()
    })
    .on_settle(|s, interval| {
        s.zoom_box = None;
        let Some(Interval { x: Some(x), y: Some(y) }) = interval else {
            return Rebuild::none();
        };
        s.x = s.x.clone().with_domain_interval((x[0], x[1]));
        s.y = s.y.clone().with_domain_interval((y[0], y[1]));
        Rebuild::tree(CHART)
    })
    .active(|s| s.mode == Mode::Zoom);
```

`aspect(Aspect::Area)` keeps the box at its area's shape, so that a zoom scales both axes by one
factor, and `aspect(Aspect::Ratio(1.0))` keeps it square. A `PanZoom` restores the domains at its
first gesture on a double-click, which can come after a box zoom, so give it the app's home scales
with `PanZoom::with_home`.

### Keyed areas

```rust
let ranges = KeyedIntervalBrush::xy("ranges")
    .areas(|s: &ChartState| s.panels.iter().map(|p| (p.id, p.plot)).collect())
    .x_scale(|s, panel| s.x_scale(panel))
    .y_scale(|s, panel| s.y_scale(panel))
    // One selection for the figure, which shows in the panel where it was drawn.
    .value(|s, panel| s.brush.filter(|(at, _)| at == panel).map(|(_, interval)| interval))
    .on_change(|s, panel, interval| {
        s.brush = interval.map(|interval| (*panel, interval));
        Rebuild::scene(CHART)
    })
    .on_settle(|_, _, _| Rebuild::tree(CHART));
```

A keyed brush's `areas` returns each area's key and rectangle, so one brush serves every panel of
a facet grid, including a grid whose panel count changes. Its scales and callbacks receive the
area's key, and the bindings choose how the panels combine:

- For one selection for the figure, store one `Option<(key, Interval)>`, and return the interval
  from `value` only for its panel, so a box drawn in another panel replaces it.
- For one selection for each panel, store a map from key to interval.
- For one range that every panel shows, return the same interval from every panel's `value`.

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
  and during a drag, and provides the selected text to the host's copy shortcut. While a press that
  it didn't take is down, such as a pan's, it requests no cursor, so the press's owner chooses it.
  The cursor is `CursorIcon::VerticalText` for a label turned within 45 degrees of vertical, and a
  drag keeps its anchor label's cursor.
