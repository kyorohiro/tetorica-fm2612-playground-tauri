"""Import an unmodified Playground release; pin archive and extracted file hashes."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import stat
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


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
            if entry.is_dir():
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
              for p in (root / 'dist').rglob('*') if p.is_file()}
    if actual != lock['files']:
        raise ValueError('dist differs from release.lock.json; import the pinned ZIP again')
    return lock


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('zip', nargs='?', type=Path)
    parser.add_argument('--version', help='Required when intentionally selecting a new release')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    if args.check:
        lock = check(ROOT)
        print(f"Verified Playground {lock['version']}: {len(lock['files'])} files")
        return
    if not args.zip:
        parser.error('provide a local release ZIP')
    sha = digest(args.zip.read_bytes())
    previous = ROOT / 'release.lock.json'
    if not args.version:
        lock = json.loads(previous.read_text())
        if sha != lock['sha256']:
            raise ValueError('ZIP checksum differs from pinned release; use --version to select a new release')
        version = lock['version']
    else:
        version = args.version
    with tempfile.TemporaryDirectory(prefix='.import-', dir=ROOT) as temporary:
        staging = Path(temporary) / 'dist'
        staging.mkdir()
        hashes = unpack(args.zip, staging)
        lock = {'version': version, 'archive': args.zip.name, 'sha256': sha, 'files': hashes}
        destination = ROOT / 'dist'
        if destination.exists():
            destination.rename(Path(temporary) / 'previous-dist')
        staging.rename(destination)
        previous.write_text(json.dumps(lock, indent=2, sort_keys=True) + '\n')
    print(f'Imported {version}: {len(hashes)} files, SHA256 {sha}')


if __name__ == '__main__':
    main()
