#!/usr/bin/env python3
import json
import re
import sys
from pathlib import Path


CRATE_NAME_OVERRIDES = {
    "avenger-typst": "avenger_typst",
}


def parse_tree(path: Path) -> set[tuple[str, str]]:
    packages = set()
    for line in path.read_text().splitlines():
        item = line.strip()
        if not item or item.endswith("(*)"):
            continue
        item = item.replace(" (proc-macro)", "")
        match = re.match(r"^([^ ]+) v([^ ]+)(?: |$)", item)
        if match:
            packages.add((match.group(1), match.group(2)))
    return packages


def package_to_crate_name(package_name: str) -> str:
    return CRATE_NAME_OVERRIDES.get(package_name, package_name.replace("-", "_"))


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: rank_typst_unique_bloat.py <results-dir>", file=sys.stderr)
        return 2

    results_dir = Path(sys.argv[1])
    typst = parse_tree(results_dir / "typst-tree.txt")
    typst_packages = sorted(typst)

    bloat = json.loads((results_dir / "bloat-typst-crates.json").read_text())
    crate_sizes = {entry["name"]: int(entry["size"]) for entry in bloat["crates"]}

    rows = []
    for package, version in typst_packages:
        crate_name = package_to_crate_name(package)
        text_bytes = crate_sizes.get(crate_name, 0)
        note = "" if crate_name in crate_sizes else "not attributed by cargo-bloat"
        rows.append((text_bytes, package, version, crate_name, note))

    rows.sort(key=lambda row: (-row[0], row[1], row[2]))

    output = results_dir / "typst-bloat.tsv"
    with output.open("w") as f:
        f.write("rank\tpackage\tversion\tcrate\ttext_bytes\ttext_kib\tnote\n")
        for rank, (text_bytes, package, version, crate_name, note) in enumerate(rows, 1):
            f.write(
                f"{rank}\t{package}\t{version}\t{crate_name}\t"
                f"{text_bytes}\t{text_bytes / 1024:.1f}\t{note}\n"
            )

    total_text = sum(row[0] for row in rows)
    attributed_count = sum(1 for row in rows if row[0] > 0)
    print(f"typst_packages={len(rows)}")
    print(f"typst_attributed_packages={attributed_count}")
    print(f"typst_text_bytes={total_text}")
    print(f"typst_text_kib={total_text / 1024:.1f}")
    print(f"wrote={output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
