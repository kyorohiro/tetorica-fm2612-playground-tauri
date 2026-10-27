#!/usr/bin/env python3
"""Pin the verbatim application MSM5205 core; --check works without a parent checkout."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FILES = ('msm5205_pcm.c', 'msm5205_pcm.h', 'LICENSE', 'NOTICE.md')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, help='optional hello_ymfm source root for update/comparison')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    output = ROOT / 'src-tauri/native-msm5205'
    pin = output / 'sources.json'
    if args.check:
        manifest = json.loads(pin.read_text())
        if (manifest.get('schemaVersion') != 1
                or manifest.get('source') != 'hello_ymfm/native/msm5205'
                or set(manifest.get('sha256', {})) != set(FILES)):
            raise SystemExit('Invalid Native MSM5205 source pin')
        for name in FILES:
            target = output / name
            if digest(target.read_bytes()) != manifest['sha256'][name]:
                raise SystemExit('Native MSM5205 source hash differs: ' + name)
            if args.source and target.read_bytes() != (args.source / 'native/msm5205' / name).read_bytes():
                raise SystemExit('Native MSM5205 source differs from comparison checkout: ' + name)
        print('Native MSM5205 source snapshot: verified (4 verbatim C/header/license/provenance files)')
        return
    source = (args.source or ROOT.parents[1]) / 'native/msm5205'
    contents = {name: (source / name).read_bytes() for name in FILES}
    output.mkdir(parents=True, exist_ok=True)
    for name, data in contents.items():
        target = output / name
        if not target.exists() or target.read_bytes() != data:
            target.write_bytes(data)
    manifest = {'schemaVersion': 1, 'source': 'hello_ymfm/native/msm5205',
                'sha256': {name: digest(data) for name, data in contents.items()}}
    pin.write_text(json.dumps(manifest, indent=2) + '\n')
    print('Native MSM5205 source snapshot: updated (verbatim, BSD-3-Clause)')


if __name__ == '__main__':
    main()
