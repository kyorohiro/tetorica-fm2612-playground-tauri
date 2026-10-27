#!/usr/bin/env python3
"""Pin the verbatim application CD-DA core; --check works without a parent checkout."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FILES = ('cdda_pcm.c', 'cdda_pcm.h', 'LICENSE')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, help='optional hello_ymfm source root for update/comparison')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    output = ROOT / 'src-tauri/native-cdda'
    pin = output / 'sources.json'
    if args.check:
        manifest = json.loads(pin.read_text())
        if (manifest.get('schemaVersion') != 1
                or manifest.get('source') != 'hello_ymfm/native/cdda'
                or set(manifest.get('sha256', {})) != set(FILES)):
            raise SystemExit('Invalid Native CD-DA source pin')
        for name in FILES:
            target = output / name
            if digest(target.read_bytes()) != manifest['sha256'][name]:
                raise SystemExit('Native CD-DA source hash differs: ' + name)
            if args.source and target.read_bytes() != (args.source / 'native/cdda' / name).read_bytes():
                raise SystemExit('Native CD-DA source differs from comparison checkout: ' + name)
        print('Native CD-DA source snapshot: verified (3 verbatim C/header/license files)')
        return
    source = (args.source or ROOT.parents[1]) / 'native/cdda'
    contents = {name: (source / name).read_bytes() for name in FILES}
    output.mkdir(parents=True, exist_ok=True)
    for name, data in contents.items():
        target = output / name
        if not target.exists() or target.read_bytes() != data:
            target.write_bytes(data)
    manifest = {'schemaVersion': 1, 'source': 'hello_ymfm/native/cdda',
                'sha256': {name: digest(data) for name, data in contents.items()}}
    pin.write_text(json.dumps(manifest, indent=2) + '\n')
    print('Native CD-DA source snapshot: updated (verbatim, BSD-3-Clause)')


if __name__ == '__main__':
    main()
