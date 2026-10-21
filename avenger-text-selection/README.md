# avenger-text-selection

Text selection for an avenger app: an overlay that selects the text of titles and labels under a
drag, and copies it as the labels show it.

```rust
let app = app.with_overlay(Arc::new(TextSelection::new()))?;
```

- A press on any label starts a selection there, and a drag extends it across the labels that
  the chart draws, in the order it draws them, with the labels between whole, such as a
  legend's entries. Off text, the drag's end falls in the nearest label. Two presses in a row
  select by word, and three by line.
- A press off every label and Escape clear the selection.
- The overlay draws the selection as translucent shapes over the text, turned with a rotated
  label and cut by the label's clip. `with_highlight(color)` sets their color.
- Over selectable text, and while a drag selects, it calls for a text cursor.
- It offers the selected text to copy, as the labels show it, so a label's emphasis copies as
  its text, and an equation, which selects whole, copies as its Typst markup with its `$`s. A
  tab joins a label that goes on from the line where the one before it ends, at that line's
  height and to its right, and a newline joins the rest, so the text pastes into a spreadsheet
  as cells. The host writes the text to the clipboard on the copy shortcut.

`TextSelection::new()` selects the text of every label in the chart, whether its text mark
takes hits or not, so it needs nothing of the chart. It also selects the text of the overlays
added before it, such as a pinned tooltip, so add it after them.

A press on the text of an overlay beneath, such as a pinned tooltip over a map, is the
selection's alone, so the map doesn't pan under it. A press on the scene's own text still reaches
the app's streams, which can act on their text, as a legend's toggle does, so text inside a
pannable plot selects and pans at once.

A selection stays among the labels of the layer it starts in, the scene's or the overlays', and
keeps its labels through scene rebuilds. It ends when the label it started in shows other text
or nothing draws it.
