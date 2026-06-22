# Text Mark Follow-Up

## Current State

`Text<C>` is a built-in authoring mark in `avenger-chart-marks`, and
`Text<Cartesian>` has Cartesian position channels and a render implementation
in `avenger-chart-cartesian`.

The mark-effects v1 work added text-specific render-stage support:

- `.adjust(...)` and `.adjust_transform(...)` on `Text<Cartesian>`,
- text item channels for position, offsets, angle, font/layout properties,
  leader styling, `text`, and `defined`,
- cached text measurement for adjustment transforms,
- derived `Text` output from supported primitive source marks.

The mark-effects test suite also has a test-only fixed-label transform that
uses text measurement and a base plot-area scene. It exists to validate the
adjustment-transform plumbing and visual baselines, not as a public text API.

## Remaining Work

The remaining text work is higher-level label behavior and coordinate-specific
semantics, not the existence of the Cartesian mark.

Useful next steps:

- richer collision-aware label placement with label-label avoidance,
  priorities, candidate positions, and named obstacle sets,
- shared label-placement outputs that can drive both text and leader lines in a
  future compound mark such as `LabeledPoints`,
- text-specific legend rendering if text visual channels should appear in
  legends,
- `Text<Polar>` position channels and angle semantics,
- polar-aware orientation transforms such as keep-upright text after
  `Text<Polar>` exposes scaled polar coordinates and local display bases.

## Related Notes

- [`mark-effects.md`](mark-effects.md) tracks post-v1 label placement,
  compound-label, scene-access, and hit-testing questions.
- [`polar-geometry-space.md`](polar-geometry-space.md) tracks `Text<Polar>`
  coordinate-space versus display-space semantics.
