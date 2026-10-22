#!/usr/bin/env python3
"""Bundle the matching application sources for rebuilding statically linked LGPL cores."""
import hashlib
import json
from pathlib import Path
import stat
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PREFIX = 'tetorica-playground-source/'
EXCLUDED = {'.git', 'node_modules', 'target', 'gen', '__pycache__', '.DS_Store', '__MACOSX', '.env', '.npmrc'}
ROOT_FILES = ['.gitattributes', 'package.json', 'package-lock.json', 'README.md', 'NATIVE-SOURCES.md', 'release.lock.json']
DIRECTORIES = ['audio-sidecar', 'desktop', 'dist', 'scripts', 'tests', 'assets', '.github']


def collect(root):
    """Use an explicit allowlist: never include credentials, build outputs, or this ZIP."""
    paths = []

    def visit(path):
        if path.name in EXCLUDED:
            return
        if path.is_symlink():
            raise ValueError(f'Source tree contains a symbolic link: {path.relative_to(root)}')
        if path.is_dir():
            for child in sorted(path.iterdir()):
                visit(child)
        elif path.is_file():
            paths.append(path)

    for name in ROOT_FILES:
        path = root / name
        if not path.is_file():
            raise ValueError(f'Missing rebuild source: {name}')
        visit(path)
    for name in DIRECTORIES:
        visit(root / name)
    # Include native source snapshots, Rust sources, icons, Cargo.lock, build.rs,
    # Tauri configuration and entitlements. Exclude target/generated bindings.
    for path in sorted((root / 'src-tauri').iterdir()):
        if path.is_file() or path.name in {'src', 'icons', 'capabilities'} or path.name.startswith('native-'):
            visit(path)
    return sorted(paths, key=lambda p: p.relative_to(root).as_posix())


def prepare(root=ROOT):
    paths = collect(root)
    destination = root / 'native-source-bundle'
    destination.mkdir(exist_ok=True)
    package = json.loads((root / 'package.json').read_text())
    release = json.loads((root / 'release.lock.json').read_text())
    contents = {path.relative_to(root).as_posix(): path.read_bytes() for path in paths}
    hashes = {name: hashlib.sha256(data).hexdigest() for name, data in contents.items()}
    manifest = {'schemaVersion': 1, 'applicationVersion': package['version'],
                'frontendArchiveSha256': release['sha256'], 'files': hashes}
    manifest_bytes = (json.dumps(manifest, indent=2, sort_keys=True) + '\n').encode()
    archive = destination / 'application-sources.zip'
    with tempfile.TemporaryDirectory(prefix='.sources-', dir=root) as temporary:
        candidate = Path(temporary) / archive.name
        with zipfile.ZipFile(candidate, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=6) as output:
            def add(name, data, executable=False):
                info = zipfile.ZipInfo(PREFIX + name, date_time=(1980, 1, 1, 0, 0, 0))
                info.create_system = 3
                info.external_attr = (stat.S_IFREG | (0o755 if executable else 0o644)) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                output.writestr(info, data)
            for path in paths:
                name = path.relative_to(root).as_posix()
                add(name, contents[name], bool(path.stat().st_mode & 0o111))
            add('SOURCE-MANIFEST.json', manifest_bytes)
        if not archive.exists() or candidate.read_bytes() != archive.read_bytes():
            candidate.replace(archive)
    (destination / 'README.md').write_bytes((root / 'NATIVE-SOURCES.md').read_bytes())
    (destination / 'SOURCE-MANIFEST.json').write_bytes(manifest_bytes)
    print(f'Prepared Native application sources: {len(paths)} files, SHA256 {hashlib.sha256(archive.read_bytes()).hexdigest()}')
    return archive, manifest


if __name__ == '__main__':
    prepare()
