#!/usr/bin/env python3
"""Synchronize/check the native chip snapshot without building or playing audio."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil

PSG_ROOT = "third_party/tetorica_sega_psg"
PSG_LICENSE = f"{PSG_ROOT}/LICENSE"

FILES = [
    "src/ymfm.h", "src/ymfm_fm.h", "src/ymfm_fm.ipp", "src/ymfm_opn.h",
    "src/ymfm_opn.cpp", "src/ymfm_adpcm.h", "src/ymfm_adpcm.cpp",
    "src/ymfm_ssg.h", "src/ymfm_ssg.cpp", f"{PSG_ROOT}/segapsg.h", f"{PSG_ROOT}/segapsg.cpp",
    "src/chip_state.h", "wasm/ymfm_wasm_interface.h", "wasm/ym2612_wasm.cpp",
    "wasm/segapsg_wasm.cpp", "LICENSE", PSG_LICENSE,
]


def snapshot_name(source_path):
    # Keep the project PSG license distinct from the ymfm LICENSE in this flat snapshot.
    return "LICENSE.segapsg" if source_path == PSG_LICENSE else Path(source_path).name


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify hashes without changing files")
    parser.add_argument("--source", type=Path, help="hello_ymfm checkout to compare or synchronize")
    args = parser.parse_args()
    destination = Path(__file__).resolve().parents[1] / "src-tauri/native-genesis"
    manifest_path = destination / "sources.json"
    if args.check:
        manifest = json.loads(manifest_path.read_text())
        for source_path, expected in manifest["sha256"].items():
            target = destination / snapshot_name(source_path)
            if digest(target) != expected:
                raise SystemExit(f"Native chip snapshot changed: {target}")
            if args.source and digest(args.source / source_path) != expected:
                raise SystemExit(f"Native chip snapshot differs from upstream: {source_path}")
        print(f"Native Genesis snapshot verified ({len(manifest['sha256'])} files)")
        return
    if not args.source:
        parser.error("synchronizing requires --source /path/to/hello_ymfm")
    source = args.source.resolve()
    hashes = {name: digest(source / name) for name in FILES}
    destination.mkdir(parents=True, exist_ok=True)
    for name in FILES:
        shutil.copyfile(source / name, destination / snapshot_name(name))
    manifest_path.write_text(json.dumps({"source": "hello_ymfm", "sha256": hashes}, indent=2) + "\n")
    print(f"Native Genesis snapshot synchronized ({len(FILES)} files)")


if __name__ == "__main__":
    main()
