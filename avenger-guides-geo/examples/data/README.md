# World gallery data

The `world_gallery` example reads these files. Git LFS stores them, so run
`git lfs pull` when they are pointer files. Rebuild them from their pinned
sources with:

```sh
uv run tools/geo-world-data/prepare.py
```

| File | Rows | Geometry | Columns | Source | License |
| --- | --- | --- | --- | --- | --- |
| `countries.arrows` | 177 | Native multipolygons, separated coordinates | `name`, `continent` | [Natural Earth](https://www.naturalearthdata.com/) 1:110m admin 0 countries, as [geoarrow-data](https://github.com/geoarrow/geoarrow-data) v0.2.0 publishes them | Public domain |
| `plate_boundaries.arrows` | 241 | WKB lines | `name`, `subduction` | PB2002 plate boundaries by Peter Bird, converted by Hugo Ahlenius, Nordpil, in [fraxen/tectonicplates](https://github.com/fraxen/tectonicplates) | [ODC-BY 1.0](https://opendatacommons.org/licenses/by/1-0/) |
| `earthquakes.arrows` | 3,729 | Native points, interleaved coordinates | `magnitude`, `depth_km`, `time` | The [USGS earthquake catalog](https://earthquake.usgs.gov/fdsnws/event/1/), magnitude 6 and larger, 2000 through 2024 | Public domain |

The earthquakes are sorted by magnitude, largest first, so that small ones draw
on top.

## Credits

- Natural Earth. Free vector and raster map data at naturalearthdata.com.
- Peter Bird (2003), An updated digital model of plate boundaries, Geochemistry,
  Geophysics, Geosystems, 4(3), 1027, doi:10.1029/2001GC000252. Converted to
  GIS formats by Hugo Ahlenius, Nordpil.
- U.S. Geological Survey, Earthquake Hazards Program.
