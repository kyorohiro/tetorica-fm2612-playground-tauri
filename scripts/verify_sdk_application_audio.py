#!/usr/bin/env python3
"""Read-only validation of application-audio sources in the fixed SDK release."""
import argparse
import hashlib
import json
from pathlib import Path

from import_soundchip_sdk import SNAPSHOT, APPLICATION_AUDIO_INPUTS, check_installed

ROOT = Path(__file__).resolve().parents[1]


def verify(component, root=ROOT):
    if component not in ('cdda', 'msm5205'):
        raise ValueError('Unknown SDK application audio component')
    pin = check_installed(root)
    snapshot = root / SNAPSHOT
    for name in APPLICATION_AUDIO_INPUTS:
        if not (snapshot / name).is_file():
            raise ValueError('Fixed SDK does not include complete application audio; import the published SDK source release first')
    names = (f'{component}_pcm.c', f'{component}_pcm.h', 'LICENSE', 'NOTICE.md')
    directory = snapshot / 'vendor' / ('tetorica-' + component)
    files = {}
    for name in names:
        path = directory / name
        files[name] = hashlib.sha256(path.read_bytes()).hexdigest()
    return {'component': component, 'version': pin['version'], 'archiveSha256': pin['archiveSha256'],
            'sourceCommit': pin['sourceCommit'], 'files': files}


def main(component):
    parser = argparse.ArgumentParser(description=__doc__ + ' Import a complete release with import_soundchip_sdk.py; this tool never writes sources.')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    if not args.check:
        parser.error('--check is required; fixed SDK sources cannot be rewritten by this tool')
    try:
        result = verify(component)
    except (ValueError, OSError) as error:
        raise SystemExit(str(error))
    print(f"Native {component} SDK {result['version']}: verified fixed release (4 C/header/license/provenance files)")
    return result
