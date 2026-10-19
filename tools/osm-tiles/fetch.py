# /// script
# requires-python = ">=3.10"
# dependencies = []
# ///
"""Fetch the OpenStreetMap tiles that the map tile baselines draw.

Run from anywhere in the repository:

    uv run tools/osm-tiles/fetch.py

The tiles go in avenger-render-fixtures/data/osm-tiles/{z}/{x}/{y}.png. The script skips
tiles that are already there, so it downloads each tile once. It identifies itself to the
tile server and waits between requests, as the OpenStreetMap tile usage policy asks:
https://operations.osmfoundation.org/policies/tiles/
"""

import pathlib
import time
import urllib.request

OUT = pathlib.Path(__file__).resolve().parents[2] / "avenger-render-fixtures" / "data" / "osm-tiles"
URL = "https://tile.openstreetmap.org/{z}/{x}/{y}.png"
USER_AGENT = "avenger-render-fixtures/1.0 (+https://github.com/jonmmease/avenger)"

# Zooms 0 to 2 in full, the Alps at zoom 5, and the tile west of them that the limb view shows.
TILES = (
    [(z, x, y) for z in range(3) for x in range(2**z) for y in range(2**z)]
    + [(5, x, y) for x in range(15, 19) for y in range(10, 13)]
    + [(5, 14, 11)]
)


def main() -> None:
    for z, x, y in TILES:
        path = OUT / str(z) / str(x) / f"{y}.png"
        if path.exists():
            continue
        request = urllib.request.Request(URL.format(z=z, x=x, y=y), headers={"User-Agent": USER_AGENT})
        with urllib.request.urlopen(request) as response:
            data = response.read()
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        print(f"{z}/{x}/{y}.png: {len(data)} bytes")
        time.sleep(1.0)


if __name__ == "__main__":
    main()
