#!/usr/bin/env python3
"""Pin unchanged Browser Nuked OPN2 sources and verify prefixed Native wrapping."""
import argparse
import hashlib
import json
from pathlib import Path

FILES = ['third_party/nuked-opn2/ym3438.c', 'third_party/nuked-opn2/ym3438.h',
         'third_party/nuked-opn2/LICENSE', 'third_party/nuked-opn2/README.md',
         'wasm/nuked_opn2_wasm.c', 'web/ym2612.js', 'web/ym2612-worklet-nuked.js',
         'web/segapsg.js', 'web/ym2612_dac.js']

def digest(data):
    return hashlib.sha256(data).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = Path(__file__).resolve().parents[1] / 'src-tauri'
    target = base / 'native-nuked'
    manifest = target / 'sources.json'
    if args.check:
        info = json.loads(manifest.read_text())
        for name, entry in info['files'].items():
            data = (target / entry['snapshot']).read_bytes()
            if digest(data) != entry['sha256']:
                raise SystemExit('Nuked pinned snapshot changed: ' + name)
            if args.source and (args.source / name).read_bytes() != data:
                raise SystemExit('Nuked differs from Browser reference: ' + name)
        if (target / 'LICENSE').read_bytes() != (target / 'snapshot/third_party/nuked-opn2/LICENSE').read_bytes():
            raise SystemExit('Nuked readable LGPL notice differs from source license')
        print(f'Native Nuked snapshot verified ({len(info["files"])} files)')
        return
    if not args.source:
        parser.error('--source is required to synchronize')
    files = {}
    for name in FILES:
        data = (args.source / name).read_bytes()
        relative = 'snapshot/' + name
        path = target / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        files[name] = {'snapshot': relative, 'sha256': digest(data)}
    (target / 'LICENSE').write_bytes((args.source / 'third_party/nuked-opn2/LICENSE').read_bytes())
    manifest.write_text(json.dumps({'source': 'hello_ymfm Browser Nuked OPN2 / ym2612-worklet-nuked.js',
        'license': 'LGPL-2.1-or-later (Nuked OPN2, Alexey Khokholov)',
        'adaptation': 'Pinned synthesis C and WASM wrapper remain unchanged. Native macro-prefixes ym2612 exports and all OPN2 symbols, avoiding the existing ymfm provider. The global chip type is selected once before publishing the first core, so later constructors do not race audio callbacks. native_genesis_create_nuked selects only Nuked plus the existing common PSG. Every raw address/data write advances the original 24 internal clocks. PCM sums 24 clocks and scales by 11, clock/144, as Browser. Native preserves raw Nuked idle DC; only explicit Native Stop suppresses emitted PCM until a new voice/DAC/PSG trigger. The ymfm constructor and centering path stay unchanged.',
        'files': files}, indent=2) + '\n')
    print(f'Native Nuked synchronized ({len(files)} files)')

if __name__ == '__main__':
    main()
