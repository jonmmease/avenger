#!/usr/bin/env python3
"""Refresh hashes and source count in the reviewed Tree-sitter corpus manifest.

Existing classifications and expected roots are review decisions and are
preserved verbatim. The script refuses to add or remove sources implicitly so
that changes to the corpus authority remain explicit code review events.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path


CRATE = Path(__file__).resolve().parent.parent
WORKSPACE = CRATE.parent
MANIFEST = CRATE / "tests/fixtures/tree_sitter/structural_sources.json"


def main() -> None:
    document = json.loads(MANIFEST.read_text())
    recorded = {entry["path"]: entry for entry in document["sources"]}
    discovered = {
        path.relative_to(WORKSPACE).as_posix()
        for root in document["authority"]["discovery_roots"]
        for path in (WORKSPACE / root).rglob("*.avenger")
    }

    missing = sorted(discovered - recorded.keys())
    removed = sorted(recorded.keys() - discovered)
    if missing or removed:
        details = []
        if missing:
            details.append(f"unclassified sources: {missing}")
        if removed:
            details.append(f"removed sources still recorded: {removed}")
        raise SystemExit("; ".join(details))

    sources = []
    for relative in sorted(discovered):
        entry = recorded[relative]
        entry["sha256"] = hashlib.sha256((WORKSPACE / relative).read_bytes()).hexdigest()
        sources.append(entry)
    document["sources"] = sources
    document["snapshot_source_count"] = len(sources)
    MANIFEST.write_text(json.dumps(document, indent=2) + "\n")


if __name__ == "__main__":
    main()
