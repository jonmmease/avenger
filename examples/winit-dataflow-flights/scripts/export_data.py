# /// script
# requires-python = ">=3.11"
# dependencies = ["pyreadr==0.5.6", "pyarrow==23.0.1"]
# ///
"""Reproduce the bundled, unsampled nycflights13 Parquet fixture."""
import hashlib
import json
from pathlib import Path
import tempfile
import urllib.request

import pyarrow as pa
import pyarrow.parquet as pq
import pyreadr

REVISION = "df98ef215aa8216fe0838a0b8ac5bada646d814c"
BASE = f"https://raw.githubusercontent.com/tidyverse/nycflights13/{REVISION}/data"
OUT = Path(__file__).resolve().parents[1] / "data"
OUT.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory() as tmp:
    sources = {}
    for name in ["flights", "airlines"]:
        url = f"{BASE}/{name}.rda"
        path = Path(tmp) / f"{name}.rda"
        urllib.request.urlretrieve(url, path)
        sources[name] = {"url": url, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    flights = pyreadr.read_r(str(Path(tmp) / "flights.rda"))["flights"]
    airlines = pyreadr.read_r(str(Path(tmp) / "airlines.rda"))["airlines"]
    table = pa.table({
        "flight_id": pa.array(range(len(flights)), type=pa.int32()),
        "dep_delay": pa.array(flights.dep_delay, type=pa.int32(), from_pandas=True),
        "arr_delay": pa.array(flights.arr_delay, type=pa.int32(), from_pandas=True),
        "carrier": pa.array(flights.carrier, type=pa.string()),
        "dest": pa.array(flights.dest, type=pa.string()),
        "scheduled_minute": pa.array(flights.hour * 60 + flights.minute, type=pa.int32()),
    })
    path = OUT / "flights.parquet"
    pq.write_table(table, path, compression="zstd", row_group_size=65536)
    (OUT / "airlines.json").write_text(json.dumps(dict(zip(airlines.carrier, airlines.name)), indent=2) + "\n")
    eligible = flights.dep_delay.notna() & flights.arr_delay.notna()
    manifest = {"dataset": "nycflights13", "revision": REVISION, "sources": sources,
                "source_rows": len(flights), "eligible_rows": int(eligible.sum()),
                "excluded_missing_delay": int((~eligible).sum()), "flight_id": "zero-based original source row",
                "parquet_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                "parquet_bytes": path.stat().st_size, "arrow_bytes": table.nbytes}
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest, indent=2))
