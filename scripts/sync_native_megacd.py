#!/usr/bin/env python3
"""Synchronize/check the RF5C164 native snapshot; no audio output is opened."""
import argparse
import hashlib
import json
from pathlib import Path

FILES = ["third_party/mame-rf5c164/rf5c164.h", "third_party/mame-rf5c164/rf5c164.cpp",
         "third_party/mame-rf5c164/LICENSE", "third_party/mame-rf5c164/README.md",
         "src/chip_state.h", "wasm/rf5c164_wasm.cpp"]


def native_bytes(name, data):
    return data


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--source", type=Path)
    args = parser.parse_args()
    destination = Path(__file__).resolve().parents[1] / "src-tauri/native-megacd"
    manifest_path = destination / "sources.json"
    if args.check:
        manifest = json.loads(manifest_path.read_text())
        for name, hashes in manifest["files"].items():
            if digest((destination / Path(name).name).read_bytes()) != hashes["native"]:
                raise SystemExit(f"Native RF5C164 snapshot changed: {name}")
            if args.source:
                data = (args.source / name).read_bytes()
                if digest(data) != hashes["source"] or digest(native_bytes(name, data)) != hashes["native"]:
                    raise SystemExit(f"Native RF5C164 snapshot differs from source: {name}")
        print(f"Native RF5C164 snapshot verified ({len(manifest['files'])} files)")
        return
    if not args.source:
        parser.error("synchronizing requires --source /path/to/hello_ymfm")
    destination.mkdir(parents=True, exist_ok=True)
    files = {}
    for name in FILES:
        source = (args.source / name).read_bytes()
        native = native_bytes(name, source)
        (destination / Path(name).name).write_bytes(native)
        files[name] = {"source": digest(source), "native": digest(native)}
    manifest_path.write_text(json.dumps({"source": "hello_ymfm", "files": files}, indent=2) + "\n")
    print(f"Native RF5C164 snapshot synchronized ({len(files)} files)")


if __name__ == "__main__":
    main()
