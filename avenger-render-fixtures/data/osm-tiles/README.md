# OpenStreetMap tiles

The map tile baselines draw these OpenStreetMap tiles, stored as
`{z}/{x}/{y}.png`: zooms 0 to 2 in full, a block of the Alps at zoom 5 (columns
15 to 18, rows 10 to 12), and tile 5/14/11 west of it. The tiles come from
`https://tile.openstreetmap.org/{z}/{x}/{y}.png` and were captured on
2026-10-19 with `tools/osm-tiles/fetch.py`, which downloads only the tiles that
are missing. A new case that needs other tiles adds them to the script's list.

Map data © OpenStreetMap contributors, available under the Open Database
License (ODbL). See
[OpenStreetMap copyright and attribution](https://www.openstreetmap.org/copyright).
