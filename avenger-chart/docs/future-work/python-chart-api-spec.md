# Avenger Python API Specification

## Status

Specification for `avenger`, the Python authoring and runtime package for
`avenger-chart`. This document defines the syntax and its guarantees. Every
construct was verified empirically (2026-07) against Pyright 1.1.411,
ty 0.0.7, mypy 2.1.0, and IPython/Jedi kernel completion; the probe scripts
live in the repository scratch typing lab.

```python
import avenger as av
from avenger import col, lit, v
from avenger import functions as f

import pyarrow as pa   # dtypes at API boundaries (casts, store schemas)
```

## The Three Constructs

The entire API is built from three constructs. There are no callback
parameters anywhere: no lambdas, no configuration closures.

1. **Declarations** — top-level functions and objects that create standalone
   values: params, data sources, measures, scales, axes, legends, patterns,
   tools. Declarations are ordinary Python objects held in ordinary Python
   variables.

2. **Blocks** — `with` statements that bind scoped handles: charts, groups,
   marks, views, and item frames. A child is attached to its parent at
   creation; `with` adds indentation and end-of-block validation, never
   structural mutation. Every block form has an equivalent non-block form.

3. **Accessors** — live objects hanging off marks and frames that follow one
   convention:

   - **used as an expression, an accessor reads** (`it.x + 18`,
     `bars.x` as a channel reference);
   - **called, it writes** (`bars.x("horsepower")`, `it.y(it.y - 12)`);
   - **chained, it configures** (`bars.x("hp").scale(...).axis(...)`).

Everything an author can express has exactly one compact spelling (keyword
arguments) and, where a construct has ordered or elaborate content, one block
spelling. The two are semantically identical.

A complete chart is defined by a top-level `with` block. Reuse and
parameterization are ordinary Python: wrap the block in a function that
returns the chart.

## First Example

Compact style:

```python
import avenger as av
from avenger import col, v
from avenger import functions as f

min_mpg = av.param("min_mpg", 20)

chart = av.cartesian(data=cars_df, title="Cars by horsepower")

hist = chart.group(id="histogram")
hist.filter(col("mpg") >= min_mpg)

hp = hist.bin("horsepower", maxbins=30)
count = f.count().alias("count")
hist.aggregate(count, group_by=[hp.start, hp.end, "origin"])

hist.rect(
    id="bars",
    x=hp.start, x_scale=av.Linear(nice=False, zero=False),
    x_axis=av.Axis(title="Horsepower"),
    x2=hp.end,
    y=count, y_scale=av.Linear(zero=True), y_axis=av.Axis(title="Count"),
    fill="origin", fill_scale=av.Ordinal(scheme="tableau10"),
    fill_legend=av.Legend(title="Origin"),
)

chart.save("cars.png")
```

Block style — the same chart, statement for statement:

```python
with av.cartesian(data=cars_df, title="Cars by horsepower") as chart:
    with chart.group(id="histogram") as hist:
        hist.filter(col("mpg") >= min_mpg)
        hp = hist.bin("horsepower", maxbins=30)
        count = f.count().alias("count")
        hist.aggregate(count, group_by=[hp.start, hp.end, "origin"])

        with hist.rect(id="bars") as bars:
            bars.x(hp.start).scale(av.Linear(nice=False, zero=False)).axis(title="Horsepower")
            bars.x2(hp.end)
            bars.y(count).scale(av.Linear(zero=True)).axis(title="Count")
            bars.fill("origin").scale(av.Ordinal(scheme="tableau10")).legend(title="Origin")

chart.save("cars.png")   # the with target outlives the block
```

## Block Semantics

Blocks do more than indent. The full `with` contract:

- **Creation attaches; `__enter__` binds.** A child is part of its parent the
  moment the creating call returns; `__enter__` returns that same object, so
  block and non-block forms are structurally identical.

- **Clean exit validates.** When a block exits without an exception,
  `__exit__` validates what the block defined. Because statements are
  mutations and later writes win, mid-block state is legitimately
  inconsistent — only the final state is contractual — so exit is the
  earliest sound moment for two families of checks that no single line can
  host. *Absence*: a rect that never received `x`/`y`, an `x2=` with no `x`
  anchoring the interval, a view block binding neither `x_domain` nor
  `y_domain`, a derive-text block that never set `text`, a group ending with
  transforms but no marks (warning). *Final-state coherence*: `x2_band=1.0`
  alongside an explicitly continuous final `x_scale`, `when(cond, scaled=...)`
  on a channel whose final state is unscaled, axis configuration on a channel
  that ends the block with `scale="none"` — either line alone is fine, and a
  later rewrite may fix or break the pair. Validation errors carry the
  block's captured source location, so mistakes are reported at the `with`
  statement that defined the object rather than at a later `compile()`.

- **Every "definition complete" boundary runs the same checks, earliest
  first.** A single-call constructor validates when the call returns (the
  compact keyword form has all channels in one call), a block validates at
  exit, and anything authored incrementally without a block is checked at
  the nearest enclosing block or at `compile()`. Checks that need data or
  the whole tree — column existence, scale-type inference, cousin-domain
  coordination — always remain at `compile()`.

- **Exceptional exit annotates, never masks.** If an exception is already in
  flight, `__exit__` performs no validation and never suppresses it; it
  attaches a structural breadcrumb (PEP 678 exception note) and lets the
  exception propagate. Nested blocks produce an innermost-first trail:

  ```text
  RuntimeError: scale 'band' requires a discrete domain
    while defining mark 'bars' (chart.py:42)
    while defining group 'histogram' (chart.py:38)
    while defining chart 'cars_histogram' (chart.py:35)
  ```

- **Definition sites feed later diagnostics.** Every constructor records its
  call site and every block records its `with` location and extent.
  `compile()` diagnostics — SQL parse errors, unknown columns, domain
  conflicts — cite the definition site of the node that caused them, giving
  Python authoring the same source-anchored diagnostics as the dedicated
  DSL.

- **One ambient scope: views.** A view block is the only block that changes
  the meaning of statements written inside it (transforms declared there are
  view-scoped). No other block is ambient — marks always attach through an
  explicit parent method (`chart.symbol(...)`), never by being constructed
  inside someone else's block.

- **Blocks do not seal.** After a block exits the object stays live:
  accessors may still refine it, and a partially built chart left behind by
  a failed block remains inspectable for debugging. Exit validation is a
  checkpoint, not a freeze.

## Value Slots

One rule set applies to every channel value, transform argument, guide
option, and event expression:

| Written as | Meaning | Type |
| --- | --- | --- |
| `"horsepower"` | column reference (`col("horsepower")`) | `str` |
| `col("mpg") + 1`, `f.log(...)` | expression | `Expr` |
| `42`, `1.5`, `True` | data-space literal (`lit(...)`), scaled | scalar |
| `av.v("#2563eb")`, `av.v(36)` | literal visual value, bypasses scales | `Value` |
| `min_mpg` (a `Param`) | typed runtime placeholder | `Param[T]` |
| `hp.start`, `count` | transform output / measure handle | `ChannelExpr` / `Measure` |
| `av.channel("x")`, or `bars.x` in block scope | reference to another channel of the same mark | `Expr` |

Strings that are not value slots — ids, titles, file paths, CSS payloads,
option names — are plain Python strings and are never parsed as columns.
Enum-like options are `Literal` string unions (`slots="shared"`,
`position="bottom"`, `offset="normalize"`); their docstrings enumerate the
allowed values.

Because bare strings are column references, literal colors and literal text
require `av.v(...)`:

```python
chart.symbol(x="hp", y="mpg", fill=av.v("#4682b4"))   # the color steel blue
chart.symbol(x="hp", y="mpg", fill="steel_blue_col")  # a column named steel_blue_col
```

## Expressions

### Builder Form

Expressions are DataFusion logical expressions with the operator surface of a
dataframe library:

```python
from avenger import col, lit
from avenger import functions as f

pred   = (col("mpg") >= min_mpg) & col("origin").is_in(["USA", "Europe"])
logged = f.log(col("horsepower") + 1)
label  = f.concat(col("category"), lit(": "), col("value").cast(pa.string()))
branch = f.when(col("value") > 100, lit("high")).otherwise(lit("low"))
```

- Operators `+ - * / %`, comparisons, `& | ~`, and unary `-` return `Expr`.
  `Expr.__bool__` raises, so accidental `and`/`or`/`if expr:` fails
  immediately; combine predicates with `&`, `|`, `~`.
- Methods include `.alias(name)`, `.cast(pyarrow_type)`, `.is_null()`,
  `.is_not_null()`, `.between(lo, hi)`, `.is_in(values)`, `.like(pat)`,
  `.ilike(pat)`, and `.asc()` / `.desc()` (producing sort expressions for
  `order_by=` slots).
- `avenger.functions` exposes the full DataFusion function registry — scalar,
  aggregate, and window functions — with typed signatures and docstrings.
  Aggregate functions return `Measure`; window functions return expressions
  configured with `.over(partition_by=..., order_by=...)`.
- Scalar dtypes at API boundaries (casts, store schemas) are `pyarrow`
  `DataType` values.

### SQL Form

`av.sql(...)` produces the same `Expr` from a SQL scalar expression. Declared
params are referenced as named `$name` placeholders; positional placeholders
are rejected.

```python
av.sql("log(horsepower + 1)")
av.sql("mpg >= $min_mpg and origin = $selected_origin")
```

### Template Form (Python 3.14+)

Template strings splice typed values into SQL. Interpolations are expression
holes, never text substitution:

```python
cutoff, padding = av.param("cutoff", 20), 1.5
g.filter(av.sql(t"mpg >= {cutoff} and weight + {padding} > 3000"))
```

Allowed hole values: `Param` (lowers to a placeholder), `Expr` (spliced
expression tree), and Python scalars — `int`, `float`, `bool`, `str`,
`datetime`, `date`, `Decimal`, `None` — which lower to typed literals. Any
other value, and any hole in identifier or keyword position
(`t"SELECT * FROM {table}"`), raises at construction.

### Named Outputs: The Aliased-Expression Rule

For transforms whose outputs the author names, **the aliased expression is
both the definition and the reference** — the Python variable is the handle:

```python
q1  = f.approx_percentile_cont("value", 0.25).alias("q1")
med = f.median("value").alias("med")
g.aggregate(q1, med, group_by="group")

g.rect(x=q1, x2=med, y="group", y_band=0.26)
```

Unaliased measures receive stable generated names; `.alias()` controls the
name seen in SQL and DSL export. The same rule covers `calculate` and
`window`:

```python
margin = (col("profit") / col("revenue")).alias("margin")
g.calculate(margin)

rank = f.row_number().over(partition_by="category",
                           order_by=col("value").desc()).alias("rank")
g.window(rank)
```

Keyword sugar exists for interactive work. Its result is addressed by name at
runtime (`stats["med"]`, `stats.med`), validated at compile time, and
completable in live kernels:

```python
stats = g.aggregate(group_by="group", med=f.median("value"))
stats["med"]
```

Transforms with fixed outputs return generated classes with typed fields
(`BinOutput.start`, `StackOutput.end`, `KdeOutput.value`, ...).

## Declarations

### Data

`data=` accepts any object implementing the Arrow PyCapsule interface
(pandas, polars, pyarrow, duckdb) anywhere it appears. File-backed, SQL, and
late-bound named sources are declarations:

```python
cars   = av.data(cars_df)                       # inline Arrow-compatible data
taxi   = av.csv("nyc_taxi.csv")
movies = av.parquet("movies.parquet")
top    = av.sql_data("SELECT * FROM cars WHERE mpg > 30")
named  = av.table("cars")                       # resolved against the context
```

### Params

```python
threshold = av.param("threshold", 10)                    # Param[int]
region    = av.param("region", "all", sharing="shared")  # Param[str]
x_dom     = av.raw_domain_param("x_domain")              # raw-domain param for tools/views
```

`av.param` types the handle from its default. Params are expressions: they
compose with the builder (`col("mpg") >= threshold`) and lower to typed
placeholders. `sharing=` takes `"shared"`, `"free"`, or `av.level(n)`.
Cursor state is not a parameter convention; event bindings publish it through
the explicit transactional `set_cursor(...)` effect.

### Stores And Selections

```python
picked = av.selection("picked", empty="all", combine="union")
hover  = av.store("hover",
                  fields={"id": pa.int64(), "x": pa.float64()},
                  primary_key=["id"], sharing="shared", initial=seed_df)
```

Declarations are collected automatically when a chart references them. A
declaration referenced only from SQL text (`$name`) must be registered
explicitly with `chart.add(...)`.

## Charts And Coordinate Systems

Chart constructors are per-coordinate factory functions returning
per-coordinate classes; each class exposes exactly the marks and options
valid for that coordinate system.

```python
av.cartesian(data=..., unit_aspect=1.0)                  -> CartesianChart
av.polar(data=...)                                       -> PolarChart
av.zerod(data=...)                                       -> ZeroDChart
av.geo(av.mercator(center=(-73.99, 40.75), zoom=12),
       tiles=osm, data=...)                              -> GeoChart
av.parallel(data=...)                                    -> ParallelChart
```

Geo projections are typed objects (`av.mercator(...)`, `av.equal_earth(...)`)
and tile layers are declared resources:

```python
osm = av.xyz_tiles("https://tile.openstreetmap.org/{z}/{x}/{y}.png",
                   attribution="© OpenStreetMap contributors",
                   min_zoom=1, max_zoom=18)
```

Parallel coordinates declare dimensions on the chart, then map them on marks:

```python
p = av.parallel(data=metrics)
p.dimension("speed", axis=av.Axis(title="Speed"))
p.dimension("tier", scale=av.Point(), axis=av.Axis(title="Tier"))
p.line(dimensions={"speed": "speed", "tier": "tier"}, stroke="group")
```

### Chart-Level Configuration

Configuration objects are typed keyword values on the constructor, each also
settable later through an equivalently named method:

```python
chart = av.cartesian(
    data=cars_df,
    id="sales_overview",       # structural name: DSL export, event paths
    title=av.Title("Sales", align="center", span="plot", syntax="plain"),
    subtitle="2015 season",
    canvas=av.Canvas(width=900, height=480),
    plot_area=av.PlotArea(width=640),
    margins=av.Margins(top=10, right=20, bottom=10, left=20),   # or margins=20
    resize=av.Resize(width="fill", height="fixed"),
    guide=av.Guide(background_fill=av.v("#ffffff")),
    formatting=av.Formatting(locale="en-US"),
    time=av.TimeContext(zone="America/New_York", week_start="monday"),
    theme=av.css_file("theme.css"),
    debug=av.Debug(layout=False, allocation=False),
)
```

Chart-wide defaults for scales, axes, and legends take the same value objects
used at channel level, applied by scale or guide type:

```python
chart.scale_defaults(av.Linear(nice=True), av.Band(padding_inner=0.1))
chart.axis_defaults(av.Axis(grid=True))
chart.legend_defaults(av.Legend(position="right"))
```

## Marks

Marks are created by chart and group methods; the mark attaches on creation
and is returned. Standalone constructors (`av.symbol(...)`) build unattached
marks for helper functions and `chart.mark(...)`; both spellings share one
generated signature.

```python
pts = chart.symbol(
    id="points",
    x="horsepower", y="mpg",
    fill="origin", size=60, shape="cylinders",
    opacity=0.85, stroke=av.v("#111827"), stroke_width=1.0,
    zindex=2, visible=col("mpg").is_not_null(),
)
```

Mark availability by coordinate system (each is a method on that chart
class):

| Coordinate | Marks |
| --- | --- |
| Cartesian | `symbol, rect, rule, line, area, text, image, trail, path, uniform_raster_2d, subplot, box_plot, violin` |
| Polar | `symbol, line, text, rule` and the polar arc family |
| ZeroD | `symbol, text` (no position channels) |
| Geo | `symbol` (lon/lat), `line`, `geo_shape`, `uniform_raster_2d` |
| Parallel | `line, symbol` (dimension-mapped) |

Base options on every mark: `id`, `data=` (mark-level source, including a
`Store`), `unit_data=True`, `visible=`, `zindex=`, `details=[...]`, and
`facet_scope=` (`"filtered" | "broadcast" | av.level(n)`).

## Channel Options

Every channel a mark carries appears in the mark's keyword signature as the
channel keyword plus one suffixed keyword per option that channel supports on
that mark:

```python
chart.rect(
    x="category",
    x_scale=av.Band(domain=["A", "B", "C"]),
    x_axis=av.Axis(title="Category", grid=False),
    x_domain_contribution="exclude",
    x2=av.channel("x"), x2_band=1.0,
    y=0.0, y_scale=av.Linear(domain=(0, 100)), y_axis=av.Axis(title="Value"),
    y2="value",
    fill=av.v("#4682b4"),
)
```

The suffix vocabulary:

| Keyword | Meaning | Example |
| --- | --- | --- |
| `x=` | channel value | `x="hp"`, `x=hp.start` |
| `x_scale=` | scale (object, kind string, or `"none"` to disable scaling) | `x_scale=av.Linear(nice=False)`, `x_scale="band"`, `x_scale="none"` |
| `x_axis=` | axis configuration | `x_axis=av.Axis(title="HP")` |
| `x_band=`, `x2_band=` | band position boundary | `x2_band=1.0` |
| `fill_legend=` | legend configuration | `fill_legend=av.Legend(title="Origin")` |
| `fill_when=` | ordered conditional branches | see [Conditionals](#conditionals) |
| `x_domain_contribution=` | automatic domain participation (`"infer"` or `"exclude"`) | `x_domain_contribution="exclude"` |
| `x_domain_scope=`, `x_domain_group=` | domain coordination | `x_domain_scope="shared"` |
| `x_transform_scope=` | transform sharing scope | `x_transform_scope=av.level(1)` |

Which keywords exist is decided per (coordinate, mark, channel): position
channels carry `_axis`/`_band`; legendable channels carry `_legend`;
`stroke_width` carries `_scale` but no `_legend`; a line mark has no `x2=`; a
raster mark's `x=` accepts only `RasterDim`; a subplot mark positions through
`subplot_x=` and `plot_width=`. Unknown keywords and wrong value types are
static errors, and completion inside a mark call lists exactly what that mark
supports.

### Channel Accessors

Every mark instance carries one accessor per channel. Accessors obey the
read/write/configure convention:

```python
pts = chart.symbol(x="horsepower", y="mpg", fill="origin")

pts.x(col("horsepower") + 1)                      # call: set the value
pts.x.scale(av.Linear(nice=False)).axis(title="Horsepower")   # chain: configure
pts.x2(pts.x).band(1.0)                           # read: pts.x is a channel reference
```

Accessor methods mirror the suffix keywords one-to-one and lower
identically; later writes win over creation keywords:

| Accessor | Suffix keyword |
| --- | --- |
| `pts.x(value)` | `x=value` |
| `pts.x.scale(s)` / `pts.x.scale("band")` | `x_scale=...` |
| `pts.x.no_scale()` | `x_scale="none"` |
| `pts.x.axis(a)` / `pts.x.axis(title=...)` | `x_axis=...` |
| `pts.x.band(v)` | `x_band=v` |
| `pts.fill.legend(title=...)` | `fill_legend=...` |
| `pts.fill.when(cond, value=...)` (repeatable, ordered) | `fill_when=[...]` |
| `pts.x.domain_scope("shared")`, `.domain_group(...)` | `x_domain_scope=`, `x_domain_group=` |

Configuration methods accept either the value object or its keyword options
inline: `.axis(av.Axis(title="HP"))` and `.axis(title="HP")` are equivalent.

The read/write duality is ordinary Python object mechanics, not context
sensitivity: an accessor's class subclasses `Expr`, so in operator or
argument position the object denotes a reference to its channel (operator
methods build expression nodes), while the class's `__call__` records a
write. Python evaluates call arguments before the call, so
`pts.x2(pts.x).band(1.0)` first lowers `pts.x` to a channel-reference node
and then stores it as `x2`'s value — and in adjust blocks, where reads
denote the pre-adjustment item, `it.x(it.x + 18)` unambiguously means
"new x = original x + 18". Read-only frames (a derive block's `label.item`)
use expression-only channel classes without `__call__`, so writing to a
source item is a static error.

### Mark Blocks

A mark opened as a `with` block groups its channel statements; required
channels are validated when the block closes, so errors are reported at the
block that caused them:

```python
with hist.rect(id="bars") as bars:
    bars.x(hp.start).scale(av.Linear(nice=False, zero=False)).axis(title="Horsepower")
    bars.x2(hp.end)
    bars.y(count).scale(av.Linear(zero=True)).axis(title="Count")
    bars.fill("origin").scale(av.Ordinal(scheme="tableau10")).legend(title="Origin")
```

Style guidance: keyword form for compact marks; block form when a mark
carries many configured channels, local transforms, views, or effects.

### Reusable Encodings

Option value objects are channel-agnostic and reusable
(`hp_axis = av.Axis(title="Horsepower")` passed to many marks). Whole
encodings are bundled through generated per-mark `TypedDict`s over the same
keywords:

```python
hp_x: av.channels.CartesianRect = {
    "x": "horsepower",
    "x_scale": av.Linear(nice=False),
    "x_axis": av.Axis(title="Horsepower"),
}
chart.rect(**hp_x, y="count")
```

### Conditionals

Ordered first-match branches, each carrying a literal `value=` or a scaled
`scaled=` payload:

```python
chart.symbol(
    x="x", y="y",
    fill="value",
    fill_when=[
        av.when(col("highlight"), value=av.v("#ff0000")),
        av.when(col("is_flagged"), scaled=col("importance") * 20),
    ],
    fill_scale=av.Linear(range=["#1d4ed8", "#22c55e"]),
    fill_legend=av.Legend(title="Status"),
)
```

In block form, `.when(...)` is called once per branch; call order is branch
order.

### Nested Positions

A nested position is a channel value; each level names its own field and
carries its own configuration:

```python
chart.rect(
    x=av.nested(
        av.NestLevel("quarter", scope="shared", axis=av.Axis(title="Quarter")),
        av.NestLevel("team", scope="free", padding_inner=0.08),
    ),
    y="value",
)
```

## Scales, Axes, And Legends

One class per scale type, each exposing that type's options plus the common
domain/range surface:

| Class | Type-specific options |
| --- | --- |
| `av.Linear` | `zero, nice, clamp, padding` |
| `av.Log` | `base, nice, clamp` |
| `av.Pow` | `exponent, zero, nice, clamp` |
| `av.Sqrt` | `zero, nice, clamp` |
| `av.Symlog` | `constant, nice, clamp` |
| `av.TimeScale` | `nice, clamp` |
| `av.Band` | `padding_inner, padding_outer, align, round` |
| `av.Point` | `padding, align, round` |
| `av.Ordinal` | `scheme, unknown, order_by, order` |
| `av.Threshold`, `av.Quantile`, `av.Quantize` | per type |

Common keywords: `domain=` (a 2-tuple is an interval; a list is discrete
values), `range=`, `scheme=` (a `Literal` union of built-in scheme names),
`raw_domain=` (a raw-domain `Param` for tool and viewport binding), `name=`
(a shared scale name), and categorical ordering (`order_by=`,
`order="asc" | "desc"`). Every `*_scale=` keyword and `.scale(...)` method
also accepts the scale kind as a string, plus `"none"` for unscaled channels.

`av.Axis` and `av.Legend` are option bags whose expression-valued options
accept the full slot taxonomy, params included:

```python
show_grid = av.param("show_grid", True)

axis = av.Axis(
    title="Horsepower", grid=show_grid, position="bottom",
    tick_spacing=50, label_angle=0.0,
    label_expr=av.sql("format_number(value)"), label_limit=120,
    visible=True, syntax="plain",
)

legend = av.Legend(
    title="Origin", position="top-right", orientation="vertical",
    columns=2, symbol_size=80, gradient_thickness=14,
    background=av.LegendBackground(fill=av.v("#ffffff"),
                                   corner_radius=4, padding=6),
    format_number="{:.1f}", order=col("origin").asc(),
)
```

Colorbar overlays attach at chart level via `av.ColorbarOverlay(...)`.

## Transforms

Transforms apply to groups and marks in call order. Every built-in transform
is a method (`.transform(obj, scope=...)` additionally accepts transform
objects for extensions). Fixed-output transforms return typed output classes;
author-named outputs follow the aliased-expression rule.

```python
g.filter(pred)
hp    = g.bin("horsepower", maxbins=30, nice=True)          # .start .end .index
stack = g.stack("amount", group_by="category",
                sort_by=col("segment").asc(), offset="zero") # .start .end .mid
kde   = g.kde("value", group_by="group", bandwidth=0.5)      # .value
fold  = g.fold(["q1", "q2", "q3"], key="quarter", value="sales")  # .key .value .index
imp   = g.impute(key="month", value="sales", group_by="region",
                 method="mean", flag="was_imputed")          # .value .flag
tu    = g.time_unit("date", unit="month")                    # typed part fields
tf    = g.time_fill("date", interval="1 day")
tl    = g.time_levels("date", levels=[...])
lump  = g.lump("category", value=f.sum("sales"), k=8, other="Other")
g.select(["a", "b"])
g.calculate(margin)                                          # aliased expressions
g.aggregate(q1, med, group_by="group")
g.join_aggregate(total, group_by="region")
g.window(rank)
px    = g.rasterize_2d(...)                                  # see rasters
mx    = g.scalar_aggregate(max_v=f.max("value"))             # ScalarRef handles
```

Scalar aggregates return `ScalarRef` expressions usable anywhere an `Expr`
is (axis titles, predicates, downstream transforms).

Transform sharing scope:

```python
g.bin("x", maxbins=20, scope="free")
g.bin("x", maxbins=20, scope=av.level(1))
g.bin("x", maxbins=20, scope="shared")
```

## Groups

A group is the data-preparation container: ordered transforms define its
local data context; child marks and child groups inherit it. Groups take
`data=`, `scale_hint=`, `facet_scope=`, and views.

```python
box = chart.group(id="manual_box_plot",
                  scale_hint=av.ScaleHint(channel="y", type="band"))

fence = box.group()
fence.join_aggregate(fq1, fq3, group_by="group")

inliers = fence.group()
inliers.filter((col("value") >= fq1 - (fq3 - fq1) * 1.5) &
               (col("value") <= fq3 + (fq3 - fq1) * 1.5))
wlo = f.min("value").alias("wlo")
whi = f.max("value").alias("whi")
inliers.aggregate(wlo, whi, group_by="group")
inliers.rule(id="whiskers", x=wlo, x2=whi,
             y="group", y_band=0.5, y2="group", y2_band=0.5,
             stroke=av.v("#475569"))
```

Named groups and marks form the public event-path ids.

## Views And Rasters

A view binds reactive viewport state. The view handle is bound by a block;
transforms declared inside the block are view-scoped and re-evaluate when the
view changes.

```python
x_dom = av.raw_domain_param("x_domain")
y_dom = av.raw_domain_param("y_domain")

raster = chart.uniform_raster_2d(id="density")
with raster.view(av.View.cartesian(
    id="viewport", x_domain=x_dom, y_domain=y_dom,
    stale="retarget_cached", preview_cached=True,
    throttle_ms=16,
)) as view:
    px = raster.rasterize_2d(
        x="pickup_x", y="pickup_y", agg="count", by="passenger_count",
        x_bins=av.RasterBins(extent=(view.x.domain_start, view.x.domain_end),
                             bins=view.x.pixels),
        y_bins=av.RasterBins(extent=(view.y.domain_start, view.y.domain_end),
                             bins=view.y.pixels),
    )
    raster.encode(
        raster=px.raster,
        x=px.x_dim, x_axis=av.Axis(title="Pickup x"),
        y=px.y_dim, y_axis=av.Axis(title="Pickup y"),
        fill_by=px.by_dim, fill_by_legend=av.Legend(title="Passengers"),
        opacity_by_total=True,
    )
```

`view.x` and `view.y` expose `domain_start`, `domain_end`, and `pixels`
expressions. Raster dimensions (`px.x_dim`, `px.y_dim`, `px.by_dim`) have
their own type, `RasterDim`; raster position keywords accept only
`RasterDim`, and scalar-expression slots reject one. Materialization
identity and policy options are keywords on the view and transform surfaces.

`.encode(**channels)` shares the mark's keyword signature and re-encodes
after creation.

## Mark Effects

Effects use blocks that bind per-item frames. Item accessors follow the
accessor convention: used as an expression they read the item's value;
called (in an adjust block) they write the adjustment. Frames are typed per
mark geometry — a `PointItem` has `.x`, `.y`, `.size`; a `RectItem` has
`.x/.x2/.y/.y2`; all frames have `.bbox`
(`.top/.bottom/.left/.right/.width/.height`), `.datum(field)`, and a
`.channel(name)` escape hatch.

```python
pts = chart.symbol(id="points", x="x", y="y", fill=av.v("#2f6fed"))

with pts.adjust() as it:
    it.x(it.x + 18)
    it.y(it.y - 12)
```

Reads inside an adjust block reference the original, pre-adjustment item:
adjustments are simultaneous, not sequential.

Adjustment transforms return typed outputs that are re-encoded onto the mark:

```python
placed = pts.adjust_transform(av.nudge(dx=18, dy=-12))   # also av.jitter, av.dodge
pts.x(placed.x).no_scale()
pts.y(placed.y).no_scale()
```

Derived marks bind the new mark's builder; the source item frame is its
`.item` attribute (read-only expressions there — writes go to the derived
mark's own accessors):

```python
with pts.derive_text() as label:
    it = label.item
    label.x(it.x)
    label.y(it.bbox.top - 4)
    label.text(it.datum("label"))
    label.fill(av.v("#111827"))
```

Derived-mark kinds: `derive_symbol`, `derive_text`, `derive_rule`,
`derive_rect`. Effect methods exist only on marks that support them;
containers reject them statically.

## Compound Marks

Compound marks take per-part configuration objects in keyword form, and
expose one part accessor per part in block form. Part accessors carry
exactly that part's channel accessors and, where supported, effect blocks.

```python
chart.box_plot(
    x="origin", y="mpg", orientation="vertical", extent=1.5,
    box=av.BoxPlot.Box(fill="origin", opacity=0.55),
    median=av.BoxPlot.Median(stroke=av.v("#111827"), stroke_width=2),
    whiskers=av.BoxPlot.Whiskers(stroke=av.v("#374151")),
    caps=av.BoxPlot.Caps(band=(-0.2, 0.2)),
    outliers=av.BoxPlot.Outliers(fill=av.v("#ffffff"), stroke="origin"),
)
```

```python
with chart.box_plot(x="origin", y="mpg", orientation="vertical") as bp:
    bp.box.fill("origin").opacity(0.55)
    bp.median.stroke(av.v("#111827")).stroke_width(2)
    bp.caps.band((-0.2, 0.2))
    with bp.outliers.derive_text() as label:
        label.text(label.item.datum("origin"))
```

```python
chart.violin(
    x="origin", y="mpg", orientation="vertical", width_normalization="shared",
    density=av.Violin.Density(fill="origin", opacity=0.6),
    outline=av.Violin.Outline(stroke=av.v("#334155")),
    center_line=av.Violin.CenterLine(stroke=av.v("#0f172a")),
)
```

## Composition

Containers attach children through `cell(...)`, which returns the child.

### Concatenation

```python
grid = av.grid_concat(rows=2, columns=2, spacing=12,
                      widths=[av.fr(2), av.px(280)],
                      heights=[av.fit(), av.fr(1)])

overview = grid.cell(av.cartesian(data=sales),
                     row=0, column=0, column_span=2, key="overview")
overview.line(x="date", y="total")

detail = grid.cell(av.cartesian(data=sales), row=1, column=0)
detail.symbol(x="date", y="value", fill="category")
```

`av.hconcat`, `av.vconcat`, `av.grid_concat`, `av.wrap_concat`; track sizing
via `av.fr(n)`, `av.px(n)`, `av.fit()`; unfilled grid cells are holes; guide
visibility and sharing policies are options on the container.

### Facets

Facet containers are single-dimension and nest:

```python
fw = av.facet_wrap(col("origin"), data=cars,
                   columns=av.responsive(min_width=220),
                   order_by=f.median(col("mpg")).desc(),
                   slots="shared", empty_cells="hide",
                   guide=av.FacetGuide(title="Origin"))
p = fw.cell(av.cartesian())
p.symbol(x="horsepower", y="mpg", fill="cylinders")

fr = av.facet_row(col("origin"), data=cars, slots="shared")
fc = fr.cell(av.facet_column(col("cylinders"), order_by=f.median(col("mpg"))))
inner = fc.cell(av.cartesian())
inner.symbol(x="horsepower", y="mpg")
```

### Repeat

Repeat containers expose typed placeholders usable as expressions, including
in cell predicates:

```python
r = av.repeat_grid(
    rows=[av.var("mpg", title="MPG"), av.var("horsepower", title="Horsepower")],
    columns=[av.var("weight", title="Weight"), av.var("acceleration", title="Acceleration")],
    domains="matrix",
)
p = r.cell(av.cartesian(), when=r.row_id != r.column_id)
p.symbol(x=r.column, y=r.row, fill="origin")
```

### Positioned Subplots

A subplot is a data-driven mark inside another coordinate system:

```python
parent.subplot(detail_chart, x="cx", y="cy", width=220, height=160,
               key="row_id", share=av.SubplotSharing(x="shared", fill="shared"))
```

## Interaction

### Tools

```python
chart.tool(av.tools.pan_scroll_zoom(x=True, y=True, scroll_zoom=True,
                                    zoom_base=1.05, settle_exact=True))
chart.tool(av.tools.box_zoom())
chart.tool(av.tools.point_selection(selection=picked, target=pts))
chart.tool(av.tools.lasso_selection(selection=picked, target=pts))
chart.tool(av.tools.box_selection(selection=picked, resolve="union"))
chart.tool(av.tools.unit_aspect_box())
```

### Events

`chart.on(...)` builds a binding fluently; `av.event` exposes typed accessors
over the event payload; `av.stream(...)` builds bare event streams for
between-bindings.

```python
(chart.on("click", target=pts, filter=av.event.datum("value") > threshold,
          consume=True, mode="preview", settle_exact=True)
      .set_param(threshold, av.event.datum("value"))
      .update_store(hover, av.store_ops.upsert(
          id=av.event.datum("id"), x=av.event.coord("x")))
      .update_selection(picked, av.select_ops.toggle(av.event.datum("id")))
      .set_cursor("pointer"))

(chart.on("pointermove", target="plot",
          between=av.between(start=av.stream("pointerdown", target=box_mark),
                             end=av.stream("pointerup")))
      .set_param(drag_x, av.event.coord("x")))
```

`av.event` accessors: `x`, `y`, `coord(channel)`, `domain(channel)`,
`datum(field)`, `key`, `button`, `wheel_delta`, modifier flags,
canvas/window dimensions, and start/previous variants inside
between-bindings. Event names, targets (mark objects, mark ids, `"plot"`,
legend surfaces), update operations, and evaluation modes are typed unions.

## Theming, Patterns, And Text

```python
accent = av.theme_param("--accent", "#2563eb")
chart.theme(av.css("""
mark[type="symbol"] { stroke: color-mix(in oklab, var(--accent), white 30%); }
"""))
chart.theme(av.css_file("theme.css"))
```

CSS is ordinary strings and files, validated by the theme parser at compile
time. Theme params are CSS custom properties addressable from Python as
params.

Patterns are structured values:

```python
stripe = av.pattern.stripe(angle=45, spacing=12, stroke_width=2)
dots   = av.pattern.symbol(symbol="circle", size=3,
                           lattice=av.pattern.lattice(dx=8, dy=8))
chart.rect(..., fill_pattern=av.v(av.pattern.fill([stripe, dots], opacity=0.55)))
```

Typst math and markup is a `syntax="typst" | "plain"` option on every
text-bearing surface: titles, subtitles, axis titles and labels, legend
titles and labels, and text channels.

## Runtime

```python
compiled = chart.compile()          # full validation; returns CompiledChart
png = chart.png(scale=2.0)          # bytes
svg = chart.svg()
pdf = chart.pdf()
chart.save("cars.png")              # format from extension
chart.show()                        # native window where available
chart                               # Jupyter: renders via _repr_mimebundle_
w = chart.widget()                  # interactive widget
```

Named and file-backed sources register on a context for SQL and multi-chart
sharing:

```python
ctx = av.context()
ctx.register("cars", cars_df)
ctx.register_csv("taxi", "nyc_taxi.csv")
movies = ctx.sql('SELECT * FROM "movies.parquet"')
chart = av.cartesian(data=av.table("cars"))
```

Sessions drive runtime state — params, stores, selections, preview and exact
evaluation:

```python
s = chart.session(ctx)
s.set_param(threshold, 25)
s.update_store(hover, av.store_ops.clear())
img = s.png(mode="preview")
await s.png_async()
s.resize(canvas_width=800)
```

State updates group into atomic batches — one evaluation when the block
exits, and an exception inside the block discards the whole batch:

```python
with s.batch():
    s.set_param(threshold, 25)
    s.update_store(hover, av.store_ops.clear())
    s.update_selection(picked, av.select_ops.clear())
```

Serialization: `chart.to_dsl()` emits the canonical `.avenger` DSL (mark
blocks correspond statement-for-statement to DSL channel blocks);
`chart.to_json()` / `av.Chart.from_json()` round-trip the chart AST;
expressions serialize as DataFusion `LogicalExprNode`.

Reusable charts are ordinary functions that build and return a chart;
servers, galleries, and hot-reload loops call them per request.

## Compilation Target

The API constructs the DSL's AST — the six-node generic tree specified in
[chart-dsl.md](chart-dsl.md)'s "AST And Interchange Form" — through native
bindings. It never generates DSL text. The mapping is 1:1 with nothing
left over: a declaration constructs a `Decl`, a `with` block nests
`children`, an accessor write sets a prop, an expression builds a
DataFusion `Expr` tree.

```python
chart.save("cars.avenger")    # pretty-prints the AST via the canonical formatter
spec = chart.to_json()        # the JSON interchange encoding
c2 = av.from_json(spec)       # and back
```

- Saved text is byte-identical to `avenger fmt` output, so generated and
  hand-written specs diff cleanly.
- Every constructed node records the Python frame that created it; compile
  errors point at user code — the AST-level counterpart of the block
  breadcrumbs.
- The JSON form needs no engine to produce or read — any tool with a JSON
  library can consume charts this API emits.
- Text, Python, and JSON share one resolve→lower→compile pipeline; the
  surfaces cannot drift.

## Tooling Conformance

The package ships `py.typed` with generated stubs, and the API must satisfy
the following without any type-checker plugin:

1. Pyright, ty, and mypy validate every construct in this specification:
   per-mark channel keywords, accessor methods and misuse, block targets,
   item-frame geometry, `Literal` options, and expression operators.
2. Kernel-based completion (IPython/Jedi, as in stock JupyterLab) completes
   every construct, including block targets and accessor chains before a
   cell executes.

Generation rules that guarantee this:

- Mark and accessor signatures use explicit named keyword parameters; the
  per-mark `TypedDict` bundles are exported in addition, never instead.
- Accessors are plain instance attributes with class-level type annotations —
  never `@property` descriptors — and methods carry concrete return
  annotations.
- `with` targets are always a single object; `__enter__` returns the bound
  handle's concrete type. Tuple-unpacked `with` targets do not appear in the
  public API.
- Every enum-like option is a `Literal` union whose docstring enumerates the
  values.
- Runtime-dynamic handles (keyword-named transform outputs) implement
  `__dir__` and `_ipython_key_completions_`.
- `Expr.__bool__` raises with guidance to use `&`, `|`, `~`.
- The API defines no decorators and no signature transformations; reusable
  charts are ordinary functions with exact signatures.
- Docstrings are generated from the Rust doc comments; stubs, runtime
  builders, `Literal` registries, and `avenger.functions` are all emitted
  from the shared authoring schema, and CI type-checks an expected-error
  corpus with Pyright, ty, and mypy.

## Requirements

- Python 3.12 or later. Template-string SQL (`av.sql(t"...")`) requires
  Python 3.14.
- Authoring and serialization (`to_dsl`, `to_json`) are pure Python.
  `compile()`, rendering, sessions, and eager SQL validation require the
  native module (`avenger._native`).

## Construct Reference

| Construct | Spelling | Binds / returns |
| --- | --- | --- |
| Chart | `with av.cartesian(...) as chart:` (also `av.polar`, ...) | chart object; target outlives the block |
| Group | `chart.group(id=...)` | group (optional `with`) |
| Mark | `chart.symbol(x=..., ...)` / `with chart.symbol() as m:` | mark |
| Channel | `x=`, `x_scale=`, ... / `m.x(value).scale(...).axis(...)` | accessor |
| Compound part | `box=av.BoxPlot.Box(...)` / `bp.box.fill(...)` | part accessor |
| Transform | `g.bin(...)`, `g.aggregate(measure, ...)` | typed output / measures |
| View | `with mark.view(av.View.cartesian(...)) as view:` | view handle |
| Adjust | `with mark.adjust() as it:` then `it.x(it.x + 4)` | item frame |
| Derive | `with mark.derive_text() as label:` then `label.item` | derived mark |
| Facet / concat / repeat | `av.facet_wrap(...).cell(av.cartesian())` | child chart |
| State | `av.param`, `av.store`, `av.selection` | handles (auto-collected) |
| Tool / event | `chart.tool(av.tools...)`, `chart.on(...).set_param(...)` | binding |
| Render | `chart.png()`, `.save()`, `.show()`, `.session(ctx)` | bytes / session |
| Session batch | `with session.batch():` state updates | one atomic evaluation on exit |
