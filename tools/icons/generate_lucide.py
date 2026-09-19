#!/usr/bin/env python3
"""Vendor Lucide metadata/SVGs without generating per-image UI code.

Unchanged outputs retain their timestamps so regeneration preserves Cargo caches.
Use --check to validate the vendored catalog without an upstream checkout.
"""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


def write_changed(path: Path, data: bytes) -> None:
    if not path.exists() or path.read_bytes() != data:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)


def check(repo: Path) -> None:
    catalog = json.loads((repo / "apps/desktop/icon-crate/catalog.json").read_text(encoding="utf-8"))
    keys = [entry["key"] for entry in catalog]
    assert keys == sorted(set(keys)), "catalog must be sorted and unique"
    stems = {key.removeprefix("lucide-") for key in keys}
    files = {p.stem for p in (repo / "apps/desktop/ui/lucide-icons").glob("*.svg")}
    assert stems == files, "catalog and original SVG set differ"
    assert all(key.startswith("lucide-") for key in keys)
    print(f"validated {len(keys)} Lucide icons")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path, nargs="?", help="lucide-static package directory")
    parser.add_argument("repo", type=Path, nargs="?", default=Path(__file__).resolve().parents[2])
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if args.check:
        check(args.repo)
        return
    if args.source is None:
        parser.error("source is required unless --check is used")
    tags = json.loads((args.source / "tags.json").read_text(encoding="utf-8"))
    files = sorted((args.source / "icons").glob("*.svg"), key=lambda p: p.stem)
    if not files:
        raise SystemExit("no SVG files found")
    catalog = []
    for path in files:
        stem = path.stem
        if not re.fullmatch(r"[a-z0-9]+(?:-[a-z0-9]+)*", stem):
            raise SystemExit(f"unexpected icon name: {stem}")
        label = " ".join(word.capitalize() for word in stem.split("-"))
        terms = [stem, label.lower(), *[str(alias).lower() for alias in tags.get(stem, []) if str(alias).lower() != "snippet"]]
        catalog.append(dict(key=f"lucide-{stem}", label=label, terms=list(dict.fromkeys(t.strip() for t in terms if t.strip()))))
        write_changed(args.repo / "apps/desktop/ui/lucide-icons" / path.name, path.read_bytes())
    write_changed(args.repo / "apps/desktop/icon-crate/catalog.json", (json.dumps(catalog, ensure_ascii=False, indent=2) + "\n").encode("utf-8"))
    check(args.repo)


if __name__ == "__main__":
    main()
