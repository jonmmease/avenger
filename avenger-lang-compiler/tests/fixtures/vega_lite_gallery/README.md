# Vega-Lite Gallery Fixture

This fixture ports the 189 unique examples in the pinned Vega-Lite 6.4.3
gallery to authored Avenger charts. `gallery.json` preserves all 203 curated
gallery placements and is the executable progress ledger.

The charts use a checked-in, offline `vega` schema from `catalog.avenger`.
Its Parquet files are normalized from the exact Vega datasets 3.2.1 revision
recorded in `upstream.lock.json` and from named datasets embedded in the pinned
Vega-Lite specifications. The original source hashes, schemas, sources, and
per-resource licenses are retained under `provenance/`. Parquet files are stored
with Git LFS by the repository's existing `.gitattributes` rule.

## Regenerating upstream-derived artifacts

Normal builds and tests never use the network and never run the sync tool.
To perform a deliberate upstream refresh, first check out the exact revisions
from `upstream.lock.json`, then run:

```sh
uv run --python 3.12 \
  avenger-lang-compiler/tests/fixtures/vega_lite_gallery/sync_gallery.py \
  --vega-lite-root /path/to/vega-lite \
  --vega-datasets-root /path/to/vega-datasets
```

The command refuses checkouts at other revisions and verifies every used Vega
dataset against its pinned Data Package Git-blob SHA-1 before writing
anything. The provenance manifest additionally records ordinary SHA-256 file
digests.

## Progress rules

- One `gallery.json` record owns each unique upstream specification.
- Each implemented example has exactly one named `.avenger` chart and one
  reviewed Avenger PNG baseline.
- Do not replace unsupported behavior with a visually convenient
  approximation. Record the preventing capability in `MISSING_FEATURES.md`
  and the example's `blockers` array.
- Upstream PNGs under `reference/` are comparison references. Avenger
  baselines are regression goldens; cross-renderer pixel identity is not the
  acceptance criterion.
- A blocker is removed only after a focused language/runtime test and the
  affected gallery example both pass.

See `scratch/avenger-lang/vega-lite-gallery-fixture-implementation-plan.md`
for the phased implementation and completion gates.
