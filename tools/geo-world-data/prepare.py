# /// script
# requires-python = ">=3.10"
# dependencies = ["pyarrow==26.0.0", "geoarrow-pyarrow==0.3.0"]
# ///
"""Rebuild the world gallery's data in avenger-guides-geo/examples/data.

Run from anywhere in the repository:

    uv run tools/geo-world-data/prepare.py

Each source is pinned, so the files change only when a pin does. The USGS
query is a live service whose catalog changes as events are reviewed, so a
refresh can change the earthquakes.
"""

import json
import pathlib
import urllib.request

import geoarrow.pyarrow as ga
import pyarrow as pa

OUT = pathlib.Path(__file__).resolve().parents[2] / "avenger-guides-geo" / "examples" / "data"

COUNTRIES = (
    "https://raw.githubusercontent.com/geoarrow/geoarrow-data/v0.2.0/"
    "natural-earth/files/natural-earth_countries.arrows"
)
PLATE_BOUNDARIES = (
    "https://raw.githubusercontent.com/fraxen/tectonicplates/"
    "339b0c56563c118307b1f4542703047f5f698fae/GeoJSON/PB2002_boundaries.json"
)
EARTHQUAKES = (
    "https://earthquake.usgs.gov/fdsnws/event/1/query?format=geojson"
    "&eventtype=earthquake&minmagnitude=6&starttime=2000-01-01&endtime=2025-01-01"
)


def fetch(url: str) -> bytes:
    with urllib.request.urlopen(url) as response:
        return response.read()


def write(name: str, table: pa.Table) -> None:
    path = OUT / name
    with pa.OSFile(str(path), "wb") as sink, pa.ipc.new_stream(sink, table.schema) as writer:
        writer.write_table(table)
    print(f"{name}: {table.num_rows} rows, {path.stat().st_size} bytes")


def countries() -> None:
    """Natural Earth 1:110m countries, as geoarrow-data publishes them: native
    multipolygons with separated coordinates."""
    data = fetch(COUNTRIES)
    path = OUT / "countries.arrows"
    path.write_bytes(data)
    rows = pa.ipc.open_stream(data).read_all().num_rows
    print(f"countries.arrows: {rows} rows, {path.stat().st_size} bytes")


def wkt_coords(coords) -> str:
    # repr round-trips each double exactly.
    return ", ".join(f"{x!r} {y!r}" for x, y, *_ in coords)


def plate_boundaries() -> None:
    """PB2002 plate boundaries as WKB lines, with whether each is a subduction zone."""
    features = json.loads(fetch(PLATE_BOUNDARIES))["features"]
    wkt = [f"LINESTRING ({wkt_coords(f['geometry']['coordinates'])})" for f in features]
    table = pa.table(
        {
            "name": [f["properties"]["Name"] for f in features],
            "subduction": [f["properties"]["Type"] == "subduction" for f in features],
            "geometry": ga.with_crs(ga.as_wkb(ga.array(wkt)), ga.OGC_CRS84),
        }
    )
    write("plate_boundaries.arrows", table)


def earthquakes() -> None:
    """USGS earthquakes of magnitude 6 and larger as native points with
    interleaved coordinates, largest first so that small ones draw on top."""
    features = json.loads(fetch(EARTHQUAKES))["features"]
    features.sort(key=lambda f: (-f["properties"]["mag"], f["properties"]["time"]))
    coords = [f["geometry"]["coordinates"] for f in features]
    wkt = [f"POINT ({lon!r} {lat!r})" for lon, lat, *_ in coords]
    points = ga.as_geoarrow(ga.array(wkt), coord_type=ga.CoordType.INTERLEAVED)
    table = pa.table(
        {
            "magnitude": pa.array([f["properties"]["mag"] for f in features], pa.float32()),
            "depth_km": pa.array([c[2] for c in coords], pa.float32()),
            "time": pa.array(
                [f["properties"]["time"] for f in features], pa.timestamp("ms", tz="UTC")
            ),
            "geometry": ga.with_crs(points, ga.OGC_CRS84),
        }
    )
    write("earthquakes.arrows", table)


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    countries()
    plate_boundaries()
    earthquakes()
