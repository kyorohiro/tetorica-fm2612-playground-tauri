"""Fetch the selected upstream ZIP and verify it against the committed lock."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from urllib.request import urlopen, Request

from import_release import ROOT, digest


def main():
    source = json.loads((ROOT / 'release.source.json').read_text())
    lock = json.loads((ROOT / 'release.lock.json').read_text())
    url = source['url']
    if not url.startswith('https://github.com/'):
        raise ValueError('Expected an HTTPS GitHub release URL')
    with urlopen(Request(url, headers={'User-Agent': 'tetorica-playground-build'}), timeout=120) as response:
        data = response.read()
    if digest(data) != lock['sha256']:
        raise ValueError('Downloaded Playground ZIP does not match release.lock.json')
    with tempfile.TemporaryDirectory(prefix='tetorica-release-') as temporary:
        archive = Path(temporary) / lock['archive']
        archive.write_bytes(data)
        subprocess.run([sys.executable, str(ROOT / 'scripts/import_release.py'), str(archive)], check=True)
    subprocess.run([sys.executable, str(ROOT / 'scripts/import_release.py'), '--check'], check=True)


if __name__ == '__main__':
    main()
