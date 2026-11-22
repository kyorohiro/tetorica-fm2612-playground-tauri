"""Import an unmodified Playground release; pin archive and extracted file hashes."""
import argparse
import importlib.util
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import stat
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def prepare_app_version(root, version):
    if version is None:
        return None
    spec = importlib.util.spec_from_file_location(
        'app_version', Path(__file__).with_name('app_version.py'))
    helper = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(helper)
    return helper.prepare_version_update(root, version)


def is_metadata(name):
    parts = PurePosixPath(name).parts
    return '.DS_Store' in parts or '__MACOSX' in parts


def digest(data):
    return hashlib.sha256(data).hexdigest()


def unpack(archive, destination):
    hashes = {}
    with zipfile.ZipFile(archive) as source:
        entries = source.infolist()
        for entry in entries:
            path = PurePosixPath(entry.filename)
            if (path.is_absolute() or '..' in path.parts or '\\' in entry.filename
                    or ':' in entry.filename or stat.S_ISLNK(entry.external_attr >> 16)):
                raise ValueError(f'Unsafe ZIP entry: {entry.filename}')
        for entry in entries:
            if entry.is_dir() or is_metadata(entry.filename):
                continue
            name = str(PurePosixPath(entry.filename))
            if name in hashes:
                raise ValueError(f'Duplicate ZIP entry: {name}')
            data = source.read(entry)
            target = destination / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            hashes[name] = digest(data)
    if 'index.html' not in hashes or 'playground.js' not in hashes:
        raise ValueError('Expected Playground ZIP with index.html and playground.js at its root')
    return hashes


def check(root):
    lock = json.loads((root / 'release.lock.json').read_text())
    actual = {p.relative_to(root / 'dist').as_posix(): digest(p.read_bytes())
              for p in (root / 'dist').rglob('*')
              if p.is_file() and not is_metadata(p.relative_to(root / 'dist').as_posix())}
    expected = {name: sha for name, sha in lock['files'].items() if not is_metadata(name)}
    if actual != expected:
        raise ValueError('dist differs from release.lock.json; import the pinned ZIP again')
    return lock


def import_release(root, archive, version=None):
    version_update = prepare_app_version(root, version)
    if not archive.is_file():
        raise FileNotFoundError(f'Playground release ZIP not found: {archive}')
    sha = digest(archive.read_bytes())
    previous = root / 'release.lock.json'
    old_receipt = previous.read_bytes() if previous.exists() else None
    old_lock = json.loads(old_receipt) if old_receipt is not None else {}
    release_version = old_lock.get('version') if old_lock.get('sha256') == sha else f'sha256-{sha[:12]}'
    with tempfile.TemporaryDirectory(prefix='.import-', dir=root) as temporary:
        staging = Path(temporary) / 'dist'
        staging.mkdir()
        hashes = unpack(archive, staging)
        lock = {'version': release_version, 'archive': archive.name, 'sha256': sha, 'files': hashes}
        destination = root / 'dist'
        backup = Path(temporary) / 'previous-dist'
        if destination.exists():
            destination.rename(backup)
        try:
            staging.rename(destination)
            previous.write_text(json.dumps(lock, indent=2, sort_keys=True) + '\n')
            if version_update is not None:
                version_update.apply()
        except BaseException:
            if destination.exists():
                shutil.rmtree(destination)
            if backup.exists():
                backup.rename(destination)
            if old_receipt is None:
                previous.unlink(missing_ok=True)
            else:
                previous.write_bytes(old_receipt)
            raise
    print(f'Imported {release_version}: {len(hashes)} files, SHA256 {sha}')
    if version_update is not None:
        print(f'App version set to {version_update.version} (metadata and lockfiles)')
    return lock


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('zip', nargs='?', type=Path, help='Path to the Playground release ZIP')
    parser.add_argument('--check', action='store_true')
    parser.add_argument('--version', help='Set the Tauri app version (X.Y.Z) during ZIP import')
    args = parser.parse_args()
    if args.check and (args.zip is not None or args.version is not None):
        parser.error('--check cannot be combined with a ZIP or --version')
    if not args.check and args.zip is None:
        parser.error('provide the ZIP path: npm run import:release -- ./xxx.zip')
    try:
        if args.check:
            lock = check(ROOT)
            print(f"Verified Playground {lock['version']}: {len(lock['files'])} files")
        else:
            import_release(ROOT, args.zip, version=args.version)
    except (OSError, ValueError, zipfile.BadZipFile) as error:
        parser.error(str(error))


if __name__ == '__main__':
    main()
