# avenger-guides-geo

Map guides for Avenger scene graphs. `make_sphere_marks` fills a projection's
outline and strokes it. `make_graticule_marks` draws the graticule's meridians
and parallels and labels them where they meet the map's edge. Each builder
draws through an `avenger_geo::Projector` into a `SceneGroup`, as
`avenger-guides` draws axes and legends. Place both groups before the map's
data.

```rust
use avenger_geo::projections::EqualEarth;
use avenger_geo::Projection;
use avenger_guides_geo::{make_graticule_marks, make_sphere_marks, GraticuleConfig, SphereStyle};
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_typst_label::LabelEngine;

fn world_map(engine: &LabelEngine) -> Result<SceneGroup, Box<dyn std::error::Error>> {
    let mut projection = Projection::new(EqualEarth);
    let summary = projection.summarize_sphere()?;
    projection.fit([[40.0, 10.0], [640.0, 330.0]], &summary)?;
    let projector = projection.build()?;
    Ok(SceneGroup {
        marks: vec![
            make_sphere_marks(&projector, &SphereStyle::default()).into(),
            make_graticule_marks(&projector, &GraticuleConfig::default(), engine)?.into(),
        ],
        ..Default::default()
    })
}
```

`GraticuleConfig::graticule` sets the lines' steps and extents, and
`label_sides` the sides that label them: meridians on the top and bottom, and
parallels on the left and right. A line is labeled on a side where it ends. A
map with a clip extent labels the lines that cross that edge of it, as an axis
labels its ticks. A map without one, such as a world map, labels each line at
its end nearest the side, on the projection's outline. While a side's labels
overlap, every other one is left out, so the rest stay evenly spaced.

Labels are the lines' degrees, followed by `°` and N, S, E, or W. The text
engine's number provider formats the degrees with `label_format`, a pattern in
its syntax, so the engine needs one, which `LabelEngine::with_number_formatting`
sets. An empty pattern takes the provider's default.

## Example

```sh
cargo run --release -p avenger-guides-geo --example projection_gallery
```

The example draws the same routes and polygon in six projections, with
`avenger-svg`, and rasterizes the SVG to PNG. It writes both files to
`target/geo-gallery`, or to the directory given as its first argument. It uses
the bundled Lato fonts without the system's, so the output is the same on every
machine.

![Projection gallery](docs/images/projections.png)
