# Text Mark Follow-Up

## Current State

`Text<C>` is a built-in authoring mark in `avenger-chart-marks`.
`Text<Cartesian>` has Cartesian position channels and a render implementation
in `avenger-chart-cartesian`, and `Text<Polar>` has polar `r/theta` position
channels plus `GeometrySpace` orientation semantics in `avenger-chart-polar`.

The mark-effects v1 work added text-specific render-stage support:

- `.adjust(...)` and `.adjust_transform(...)` on `Text<Cartesian>`,
- `.adjust(...)` and `.adjust_transform(...)` on `Text<Polar>` after polar
  projection and coordinate-space angle conversion,
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
- public polar-aware orientation transforms such as keep-upright text if the
  test-only transform proves generally useful,
- richer local-coordinate metadata in text adjustment frames if future
  transforms need more than display-space `x`, `y`, and `angle`.

## Related Notes

- [`mark-effects.md`](mark-effects.md) tracks post-v1 label placement,
  compound-label, scene-access, and hit-testing questions.
- [`polar-geometry-space.md`](polar-geometry-space.md) records the implemented
  `Text<Polar>` coordinate-space versus display-space semantics.
