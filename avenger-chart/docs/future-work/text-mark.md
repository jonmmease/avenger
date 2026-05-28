# Text Mark Follow-Up

`Text<C>` is a built-in authoring mark in `avenger-chart-marks`, and
`Text<Cartesian>` has Cartesian position channels and a render implementation
in `avenger-chart-cartesian`.

The remaining text-mark work is about higher-level label behavior, not the
existence of the mark itself:

- collision-aware label placement,
- measured text bounds available to mark-level adjustments,
- text-specific legend rendering if text visual channels should appear in
  legends,
- coordinate-specific text positioning beyond Cartesian when the semantics are
  clear.

Smart label placement depends on the post-scale adjustment boundary described
in [adjust-api.md](adjust-api.md).
