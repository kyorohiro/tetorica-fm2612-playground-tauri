#!/usr/bin/env python3
"""Pin/check the exact HuC6280 core and BSD notices used by the Web WASM."""
import argparse
import hashlib
import json
from pathlib import Path

FILES = ["third_party/mame-huc6280/" + name for name in
         ["huc6280.cpp", "huc6280.h", "LICENSE", "README.md", "upstream/c6280.cpp", "upstream/c6280.h"]]

def digest(data):
    return hashlib.sha256(data).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1] / "src-tauri/native-huc6280"
    if args.check:
        manifest = json.loads((root / "sources.json").read_text())
        for name, entry in manifest["files"].items():
            data = (root / entry["snapshot"]).read_bytes()
            if digest(data) != entry["sha256"] or (args.source and data != (args.source / name).read_bytes()):
                raise SystemExit("HuC6280 source snapshot changed: " + name)
        if (root / "LICENSE").read_bytes() != (root / "snapshot/third_party/mame-huc6280/LICENSE").read_bytes():
            raise SystemExit("HuC6280 license alias changed")
        print(f"Native HuC6280 verified ({len(manifest['files'])} files)")
        return
    if not args.source:
        parser.error("--source /path/to/hello_ymfm is required for synchronization")
    files = {}
    for name in FILES:
        data = (args.source / name).read_bytes()
        relative = "snapshot/" + name
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        files[name] = {"snapshot": relative, "sha256": digest(data)}
    (root / "LICENSE").write_bytes((args.source / "third_party/mame-huc6280/LICENSE").read_bytes())
    (root / "sources.json").write_text(json.dumps({
        "source": "hello_ymfm/third_party/mame-huc6280 (same source as Browser WASM)",
        "upstream": "mamedev/mame ad84df9d8a14c9f2bdd996b22d2832bcdb2e4f24",
        "license": "BSD-3-Clause", "files": files}, indent=2) + "\n")
    print(f"Native HuC6280 synchronized ({len(files)} files)")

if __name__ == "__main__":
    main()
