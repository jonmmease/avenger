# Geo test data

GeoJSON fixtures for the geo coordinate system tests and examples
(`avenger-geo` / `avenger-chart-geo`; see
`avenger-chart/docs/future-work/geo-coordinate-system.md` and the
implementation plan in `scratch/geo/`).

| File | Features | Source | Notes |
| --- | --- | --- | --- |
| `us-states.json` | 52 (50 states + DC + PR) | US Census via [PublicaMundi/MappingAPI](https://github.com/PublicaMundi/MappingAPI) (the classic Leaflet choropleth dataset) | Properties: `name`, `density` (pop/mi²). Public domain (US Census). Drives the CONUS Albers choropleth baseline. |
| `ne_110m_admin_0_countries.geojson` | 177 countries | [Natural Earth 1:110m Admin 0](https://github.com/nvkelso/natural-earth-vector) | Public domain. Properties slimmed to `name`, `iso_a3`, `continent`, `pop_est`, `gdp_md` (regenerate with the `jq` filter below). Includes antimeridian-crossing geometries (Fiji, Russia) and a pole-enclosing polygon (Antarctica). |
| `ne_110m_land.geojson` | 127 land polygons | [Natural Earth 1:110m Physical: Land](https://github.com/nvkelso/natural-earth-vector) | Public domain. Basemap layer under graticule/point/line baselines. |

All files are compacted with `jq -c .`.

Regenerating the slimmed countries file:

```bash
curl -sL https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/ne_110m_admin_0_countries.geojson \
  | jq -c '{type: .type, features: [.features[] | {type, properties: {name: .properties.NAME, iso_a3: .properties.ISO_A3, continent: .properties.CONTINENT, pop_est: .properties.POP_EST, gdp_md: .properties.GDP_MD}, geometry}]}' \
  > ne_110m_admin_0_countries.geojson
```

Winding order: these files follow common GeoJSON practice, NOT strict
RFC 7946 ring orientation. The geo ingest path normalizes winding
(spherical clockwise-exterior convention) — do not pre-rewind the files.

Point data for geo marks does not live here: use the existing
`avenger-chart/tests/data/airports.parquet` (lat/lon airports) and
`nyc_taxi_2015/` (pickup lon/lat) datasets.
