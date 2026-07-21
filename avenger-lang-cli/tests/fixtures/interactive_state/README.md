# Interactive reload-state acceptance fixture

This fixture makes the document state used by the native `watch` acceptance
test observable without an inspector:

- drag inside the plot to create or move the red store-backed marker;
- click either blue source point to enlarge the amber param-backed indicator;
- save a harmless source edit and confirm both indicators survive reload;
- click the same blue point again: the amber indicator must shrink, proving
  the selection clause survived and was toggled off rather than recreated.

The chart also declares direct `float64` canvas-size parameters so the virtual
canvas frame can be resized before and after reload.
