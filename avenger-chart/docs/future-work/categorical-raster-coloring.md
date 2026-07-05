# Categorical Raster Coloring: 3D Rasters with Perceptual Color Mixing

Status (2026-07-05): research + API recommendation, not yet planned for
implementation. Companion to
[raster-arrow-representations.md](raster-arrow-representations.md) (the
categorical dimension kind this builds on) and
[geo-raster-marks.md](geo-raster-marks.md) (the mark core it extends).

## 1. Goal

Datashader-style categorical shading for aggregated rasters: a third,
categorical raster dimension (taxi vendor, species, class label), rendered as
a single image where each pixel's

- **opacity** is proportional to the total count of items in the pixel, and
- **color** is a mixture of the categories' colors weighted by their
  per-category counts.

Datashader mixes in gamma-encoded sRGB by averaging RGB components; we want
something more perceptually faithful (§3). Categorical color legends and
color-scale domain inference must fall out of the ordinary channel machinery
rather than being raster special cases.

## 2. How datashader does it (verified against source)

Aggregation: `canvas.points(df, x, y, agg=ds.by("cat", ds.count()))` produces
a 3-D `DataArray` with dims `(y, x, cat)`; `cat` must be a pandas categorical
dtype, so the category set (and plane order) is known before aggregation.
Every category gets a dense plane even if empty.

Colorizing (`transfer_functions/_colorize`):

- **Color** — a weighted average of the `color_key` colors in **gamma-encoded
  sRGB**, float math over the 8-bit components:
  `rgb = (counts @ RGB_key) / total`, cast straight back to `uint8`. There is
  no linearization and no perceptual space; this is the part we want to
  improve.
- **`color_baseline`** — before mixing, each plane is offset by
  `min(color_data)` (or an explicit baseline) so weights are non-negative;
  relevant when the per-category aggregate is something signed rather than a
  count.
- **Alpha** — computed from the **total** across planes:
  `alpha = interp(how(total), span, [min_alpha, alpha])` where `how` is the
  shared normalization enum (`eq_hist` default for `shade`, also `log`,
  `cbrt`, `linear`). `min_alpha` (default 40/255) keeps single-item pixels
  visible; NaN/empty pixels are fully transparent.
- **`rescale_discrete_levels`** — with `how='eq_hist'` and few distinct
  totals, the span is stretched downward so low counts don't wash out.
- Pixels where `total == 0` but some plane had non-NaN values fall back to an
  unweighted average of the present categories.

Two datashader behaviors worth keeping deliberately: the *alpha carries the
density* (the color never darkens with count — lightness differences come
only from mixing), and the *mix is convex* (a pure-category pixel shows the
exact category color).

## 3. Color mixing: options and recommendation

The operation is a convex combination `mix(w_k, c_k)` of K category colors
with weights `w_k = count_k / total`. Candidate spaces:

| Space | Character | Cost/pixel | Notes |
| --- | --- | --- | --- |
| Gamma sRGB (datashader) | Biased dark/muddy; hue drift | trivial | Mixing in a nonlinear encoding systematically underestimates lightness; the classic "yellow + blue = gray-green sludge". Keep only as a compatibility option. |
| Linear-light sRGB | Physically additive ("K colored lights averaged") | 1 gamma decode of the key (once) + 1 encode | Honest radiometric semantics for additive count data; brighter mixes than sRGB, but perceptual lightness/hue still drift (blue dominates perceived darkness). |
| **Oklab (recommended)** | Perceptually even lightness + good hue linearity | key→Oklab once; per pixel: weighted mean (3 FMAs/category) + one Oklab→linear→sRGB (cube + 3×3 matrix ×2) | Designed exactly for mixing/interpolation; CSS Color 4's `oklab` interpolation default. Mixes never pass through a muddy phase; a 50/50 mix reads as "halfway between" the two colors. |
| CIELAB / CAM16-UCS | Similar goal | higher | CIELAB has the blue-hue-shift defect Oklab fixes; CAM16-UCS is marginally better than Oklab at real cost. Not worth it here. |
| Kubelka-Munk pigment (Mixbox) | Paint-like subtractive mixing | high | Wrong semantics for count data (we're mixing *populations*, not pigments) and the reference implementation is CC BY-NC. Excluded. |

Decision (Jon, 2026-07-05): **mix in Oklab, not configurable** — one
mixing model, no `mix_space` option. (For datashader parity comparisons
during development, a temporary srgb path behind a test-only flag is fine;
nothing ships.) Implementation shape that keeps the per-pixel cost trivial:

1. Resolve the K category colors through the ordinal color scale once, convert
   to Oklab once per materialized raster.
2. Per pixel: `lab = Σ w_k · lab_k` (convex, so the result stays inside the
   convex hull of the key colors — usually in gamut), then one
   Oklab → linear sRGB → encode conversion, then clamp to [0,1] for the rare
   out-of-gamut corner (convexity makes serious excursions impossible;
   channel clamp is sufficient, no gamut-mapping machinery needed).
3. Alpha from the total, per §4; compose as straight (non-premultiplied) RGBA
   like the existing raster path.

Known inherent caveat (all spaces): mixing MANY categories converges on a
desaturated center — that is what averaging means, and datashader has the
same property. Perceptual mixing makes the *pairwise* mixtures faithful; it
cannot make a 6-way tie look like anything but gray-ish. Worth stating in
docs so users pick distinguishable, similar-lightness palettes (glasbey-style)
for many-category use.

## 4. Opacity semantics

Map datashader's `alpha = interp(how(total), [min_alpha, alpha])` onto our
scale system rather than porting the `how` enum: the total is just a value
run through a numeric **opacity scale** — exactly how the current
single-channel raster maps count → fill via `Sqrt` + clamp
(`taxi_geo_mercator`'s `fill.scale_with::<Sqrt>(...)`). So:

- opacity channel on the mark, driven by the per-pixel total, with any
  numeric scale (`Linear`, `Sqrt` ≈ datashader `cbrt`-ish, `Log`), a domain
  (analog of `span`), `clamp`, and a range `(min_alpha, max_alpha)` — the
  range floor IS `min_alpha`.
- `eq_hist` IS our `QuantileScale`, up to quantization: histogram
  equalization maps a value to its normalized rank (empirical CDF), and a
  quantile scale is that CDF discretized to `n` range steps. With a dense
  opacity range (64–256 steps; alpha is 8-bit anyway) a quantile scale over
  the per-pixel totals reproduces eq_hist — exact ranks, where datashader
  approximates the CDF with a histogram. What it needs is the P3
  raster-value domain inference: the quantile domain must be the CURRENT
  raster's nonzero totals, recomputed per materialization (datashader
  recomputes per shade call). Tie handling is the shared wrinkle:
  `rescale_discrete_levels` exists because eq_hist misbehaves on
  low-cardinality totals, and quantile thresholds collapse under the same
  ties — one fix covers both. `Sqrt`/`Log` cover the taxi-style cases in the
  meantime.
- Zero-total pixels → fully transparent (alpha 0 regardless of range floor).

## 5. What exists in avenger today (verified 2026-07-05)

The groundwork is further along than expected:

- **The raster schema already has a categorical dimension kind.**
  `geometry.dimensions[].coords` supports `kind: "categorical"` with
  `values: List<Utf8>` alongside `uniform` start/stop/count
  (`rasterize_2d.rs` schema, reader in
  `avenger-chart-marks/src/uniform_raster_2d.rs:771-782`).
- **The mark already renders categorical dims** — as *strips*: one
  `SceneImageMark` per category value, positioned by a band scale
  (`avenger-chart-cartesian/src/marks/uniform_raster_2d.rs:453-543`). Axis
  domain inference for categorical dims exists
  (`RasterCategoricalDimValuesUdf`). What's missing is the *overlay* render
  mode: K planes → ONE image via mixing.
- **The accumulator layout is already 3-D.** `DenseGridState` stores
  `counts[group * grid_len + cell]` — the plane-major layout categorical
  accumulation needs (`rasterize_2d.rs:1163-1290`).
- **`partition_by` produces one raster row per group** and the mark renders
  multiple rows; but rows are independent images — unsuitable for mixing,
  which needs all planes simultaneously in one RGBA build. The categorical
  dimension must live *inside one row*.
- **RGBA building is CPU-side with a 2-tier cache** (identity by buffer
  address, content by recursive hash; LRU 16). Fill colors are produced by
  applying the fill `ConfiguredScale` per cell (`coerce_cell_colors` →
  `Coercer::to_color`), then a pixel loop maps cells → RGBA with flip/opacity
  handling (`uniform_raster_2d.rs:1039-1274`).
- **Gaps**: (a) fill-scale domain is NOT inferred from raster cell values —
  the only raster-driven domains today are x/y extents (and the categorical
  axis UDF); explicit domains are used in all examples. (b) The raster mark's
  fill feeds a colorbar, not a symbol legend; ordinal scales already produce
  per-value `legend_entries()` (`ordinal.rs:120-206`), so the legend side
  needs only a real categorical channel binding.

## 6. Recommended API

### 6.1 `Rasterize2D::by(...)` — categorical plane dimension

```rust
Rasterize2D::new(col("pickup_x"), col("pickup_y"))
    .x(|x| x.extent(x_start, x_end).bins(x_bins))
    .y(|y| y.extent(y_start, y_end).bins(y_bins))
    .by(col("vendor"))          // NEW: categorical plane dimension
    .agg("count")
```

- Output: ONE raster row whose `geometry.dimensions` gains a third,
  categorical dimension (`kind: "categorical"`, `values` = the observed
  categories); `values.data` length becomes `x_bins * y_bins * K`,
  plane-major to match the accumulator.
- Category discovery: dictionary-accumulate (hash category → plane index)
  inside the UDAF and emit sorted values. The mark maps plane colors through
  the ordinal fill scale **by value, not by plane order**, so accumulator
  order, scale domain order, and legend order are decoupled — no need to know
  K up front (datashader's categorical-dtype requirement disappears).
- `by` composes with `partition_by` (planes within each partition row) and is
  orthogonal to `frame()`/CRS.
- The transform output handle grows `hist.by_dim()` exposing the categorical
  dimension column (values per plane) for channel binding, mirroring
  `hist.x_dim()`.

### 6.2 Mark channels — overlay mode

```rust
UniformRaster2DChannels::raster_with(mark, hist.raster(), |r| {
    r.x(hist.x_dim())
     .y(hist.y_dim())
     .fill_by(hist.by_dim(), |fill| {          // NEW: categorical color channel
         fill.legend(|l| l.title("Vendor"))    // palette = theme default categorical
     })
     .opacity_by_total(|o| {                   // NEW: density → alpha
         o.scale_with::<Sqrt>(|s| s.clamp(true).domain((0.0, 1.0)))
          .range((0.15, 1.0))                  // min_alpha analog
     })
})
```

Mixing is always Oklab — there is no mixing configuration on the mark.

- `fill_by(dim)` binds the fill channel to the categorical dimension VALUES
  with an ordinal color scale. Its default range is whatever the THEME's
  default categorical scheme is (Okabe-Ito in ours) — exactly like any other
  categorical color channel, no raster-specific palette. This is an ordinary
  categorical channel, so:
  - **domain inference** = unique values of the dim column — implement as a
    fill-channel `scale_domain_source` backed by the existing
    `RasterCategoricalDimValuesUdf` pattern (closing gap 5a for this case);
  - **legend** = the ordinal scale's existing `legend_entries()` → standard
    swatch/symbol legend; no raster-specific legend code.
- When `fill_by` is present the categorical dim renders in overlay mode (one
  mixed image); the existing strip mode remains what a categorical dim bound
  to x/y position produces (small-multiples strips). Binding the SAME dim to
  both a position channel and `fill_by` is an error.
- `opacity_by_total` runs the summed plane through a numeric scale to alpha
  (§4). Default if omitted: linear over the inferred max with range
  `(0.15, 1.0)`.
- Numeric-valued 3D rasters (e.g. `agg("sum")` per category) reuse
  datashader's baseline rule: weights = `value_k - min_k(values)`; an
  explicit `baseline(f64)` override can live on the raster channel config
  if signed aggregates ever need it (defer until then).

### 6.3 RGBA builder extension

New multi-plane path in `avenger-chart-marks::uniform_raster_2d`:

- Inputs: K plane slices + K resolved category colors (converted to Oklab
  once) + the opacity scale.
- Per pixel: weights from planes → total → alpha via opacity scale; convex
  Oklab mix → sRGB; straight-alpha RGBA8 as today.
- Cache keys (both tiers) extend with: category colors + opacity scale
  fingerprint — the existing content-hash machinery covers the plane data
  itself.
- Cost: O(K) per pixel; for a 440×350 half-res raster with K≤12 this is well
  under the existing single-plane build cost envelope. GPU mixing (planes as
  a texture array + a small shader) is a natural follow-up if K or resolution
  grows, and would slot into the Phase-3 GPU-residency work already tracked
  in geo-raster-marks.md.

### 6.4 Mark-level 3D input (no Rasterize2D)

Because the categorical dimension is pure schema, externally produced 3D
rasters (xarray-style `(y, x, species)` grids) work with the same mark API:
provide the raster struct with a categorical dimension and bind `fill_by` to
it. No transform involvement required — same rule as today's 2D external
rasters.

## 7. Interactions with existing systems

- **Adaptive raster/scatter + async pipeline**: unchanged — `by()` output is
  still one materialized row; keys/identity gain the `by` expr via the
  serialized spec automatically; preview retargeting stretches the mixed
  image exactly like today's.
- **Geo**: the overlay path produces one RGBA image → identity fast path and
  `warped_raster_mesh` both work unchanged; `frame()` composes.
- **Scale domain inference for color**: the categorical fill domain comes
  from the raster's dim values (post-aggregation), which equals the distinct
  values of the source column that survived upstream filters — consistent
  with how in-view filtering already affects other inferred domains.
  Cross-view stability of legend colors (categories dropping out of view
  changing color assignments) is the one UX wrinkle: recommend inferring the
  domain at the plot/data level (source column) rather than per
  materialization, which the ordinary channel-inference path (over the
  pre-aggregation dataframe) gives us for free if `fill_by` also records the
  ORIGINAL column expr as its inference source. Decision needed at
  implementation time; the API above supports either.
- **Legends**: swatch legend from the ordinal scale; the legend does not
  attempt to communicate the mixing (datashader doesn't either).

## 8. Suggested phasing

1. **P1 — transform**: `by()` on Rasterize2D (dictionary accumulation, plane
   emission, `by_dim()` handle), unit tests over plane counts/order.
2. **P2 — mark overlay mode**: `fill_by` + `opacity_by_total`, multi-plane
   RGBA builder with Oklab mixing, cache-key extension; visual baselines
   (datashader parity eyeballed during development via a temporary srgb
   path, not shipped).
3. **P3 — inference + legend**: categorical fill domain source, swatch legend
   binding, cross-view domain-stability decision.
4. **P4 — examples**: taxi-by-vendor overlay on `taxi_geo_mercator`
   (adaptive), plus a non-geo categorical example.
5. **Later**: eq_hist opacity via `QuantileScale` over the totals plane
   (rides on the P3 raster-value domain inference; add the tie/
   `rescale_discrete_levels` handling shared with quantile); GPU mixing;
   per-category aggregates other than count (baseline handling) beyond the
   datashader rule.

## 9. Decisions

Resolved (Jon, 2026-07-05):

- **Mixing: Oklab, not configurable.** No `mix_space` option ships.
- **Default palette: the theme's default categorical scheme** (Okabe-Ito in
  our theme) via the ordinary ordinal-scale default range — no
  raster-specific palette.

Still open:

- Domain-stability: infer categorical color domain from the source column
  (stable legend across pan/zoom, recommended) vs from materialized plane
  values (view-local).
- Default opacity scale/range.
