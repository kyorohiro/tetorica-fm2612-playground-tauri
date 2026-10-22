#!/usr/bin/env python3
"""Verify every committed Native source pin and optional matching source ZIP offline.

No parent checkout, package registry, compiler or audio device is required.
"""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path, PurePosixPath
import stat
import subprocess
import sys
import time
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def verify_bundle(root=ROOT):
    spec = importlib.util.spec_from_file_location('prepare_native_sources', root / 'scripts/prepare_native_sources.py')
    source = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(source)
    directory = root / 'native-source-bundle'
    archive = directory / 'application-sources.zip'
    external = (directory / 'SOURCE-MANIFEST.json').read_bytes()
    manifest = json.loads(external)
    if manifest.get('schemaVersion') != 1:
        raise ValueError('Unsupported Native source manifest version')
    files = manifest['files']
    current = {p.relative_to(root).as_posix(): p for p in source.collect(root)}
    if set(current) != set(files):
        missing, extra = sorted(set(current) - set(files)), sorted(set(files) - set(current))
        raise ValueError(f'Source bundle file set differs from the checkout: missing={missing[:5]}, extra={extra[:5]}')
    package = json.loads((root / 'package.json').read_text())
    release = json.loads((root / 'release.lock.json').read_text())
    if manifest['applicationVersion'] != package['version'] or manifest['frontendArchiveSha256'] != release['sha256']:
        raise ValueError('Source bundle application/frontend version differs from the checkout')
    expected = {source.PREFIX + name for name in files} | {source.PREFIX + 'SOURCE-MANIFEST.json'}
    with zipfile.ZipFile(archive) as zipped:
        names = zipped.namelist()
        if len(set(names)) != len(names) or set(names) != expected:
            raise ValueError('Source archive contains duplicate, missing or unmanifested entries')
        if zipped.read(source.PREFIX + 'SOURCE-MANIFEST.json') != external:
            raise ValueError('Embedded and external Native source manifests differ')
        for name, expected_hash in files.items():
            path = PurePosixPath(name)
            if path.is_absolute() or '..' in path.parts or '\\' in name:
                raise ValueError('Unsafe source archive name: ' + name)
            info = zipped.getinfo(source.PREFIX + name)
            if stat.S_IFMT(info.external_attr >> 16) != stat.S_IFREG:
                raise ValueError('Source archive member is not a regular file: ' + name)
            if digest(zipped.read(info)) != expected_hash:
                raise ValueError('Source ZIP content hash mismatch: ' + name)
            if digest(current[name].read_bytes()) != expected_hash:
                raise ValueError('Source ZIP is stale relative to the checkout: ' + name)
    if (directory / 'README.md').read_bytes() != (root / 'NATIVE-SOURCES.md').read_bytes():
        raise ValueError('Bundled Native rebuild instructions differ from the checkout')
    return {'status': 'passed', 'archive': str(archive.relative_to(root)),
            'sha256': digest(archive.read_bytes()), 'files': len(files),
            'applicationVersion': manifest['applicationVersion'],
            'frontendArchiveSha256': manifest['frontendArchiveSha256']}


def verify_pins(root=ROOT, results=None):
    scripts = sorted((root / 'scripts').glob('sync_native_*.py'))
    if not scripts:
        raise ValueError('No Native source snapshot checks found')
    # Every Native snapshot directory must have a verifier. Most checks share
    # providers as well; run the individual checks rather than guessing their schema.
    aliases = {'native-fx': 'sync_native_genesis_fx.py',
               'native-noise': 'sync_native_genesis_noise.py',
               'native-livefx': 'sync_native_genesis_livefx.py'}
    checks = {p.name for p in scripts}
    for native in sorted((root / 'src-tauri').glob('native-*')):
        if not native.is_dir():
            continue
        script = aliases.get(native.name, 'sync_' + native.name.replace('-', '_') + '.py')
        if script not in checks:
            raise ValueError('No source pin check for ' + native.name)
    if results is None:
        results = []
    for script in scripts:
        result = subprocess.run([sys.executable, str(script), '--check'], cwd=root,
                                capture_output=True, text=True, timeout=30)
        summary = result.stdout.strip()
        results.append({'script': script.name, 'status': 'passed' if result.returncode == 0 else 'failed',
                        'summary': summary, 'error': result.stderr.strip()})
        if result.returncode:
            raise ValueError(f'{script.name} failed: {result.stderr.strip() or summary}')
        print(summary)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', action='store_true', help='also verify the generated source ZIP against all current sources')
    parser.add_argument('--out', type=Path, help='write JSON on both success and failure')
    args = parser.parse_args()
    started = time.monotonic()
    report = {'schemaVersion': 1, 'deviceOpened': False, 'networkUsed': False,
              'platform': sys.platform, 'snapshots': [], 'sourceBundle': None, 'status': 'failed'}
    try:
        verify_pins(results=report['snapshots'])
        if args.bundle:
            report['sourceBundle'] = verify_bundle()
        report['status'] = 'passed'
    except Exception as error:
        report['error'] = str(error)
        print(str(error), file=sys.stderr)
    finally:
        report['elapsedSeconds'] = round(time.monotonic() - started, 3)
        if args.out:
            args.out.parent.mkdir(parents=True, exist_ok=True)
            args.out.write_text(json.dumps(report, indent=2) + '\n')
    if report['status'] != 'passed':
        return 1
    print(f'Native source verification passed: {len(report["snapshots"])} offline snapshots' +
          (f', {report["sourceBundle"]["files"]} source ZIP files' if args.bundle else ''))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
