import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location('native_sources', Path(__file__).resolve().parents[1] / 'scripts/prepare_native_sources.py')
sources = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sources)


class NativeSourceTests(unittest.TestCase):
    def fixture(self, root):
        for name in ['README.md', 'NATIVE-SOURCES.md', 'package-lock.json',
                     'dist/playground.js', 'desktop/audio-interface.js',
                     'src-tauri/Cargo.lock', 'src-tauri/build.rs', 'src-tauri/src/main.rs',
                     'src-tauri/native-nuked/ym3438.c', 'src-tauri/native-nuked/LICENSE',
                     'src-tauri/native-snes/snapshot/SPC_DSP.cpp', 'scripts/rebuild.sh']:
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('matching source: ' + name)
        (root / 'package.json').write_text('{"version":"1.2.3"}')
        (root / 'release.lock.json').write_text('{"sha256":"frontend-hash"}')

    def test_zip_contains_corresponding_rebuild_material_and_verifiable_hashes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            archive, manifest = sources.prepare(root)
            self.assertEqual(manifest['frontendArchiveSha256'], 'frontend-hash')
            self.assertEqual(manifest['applicationVersion'], '1.2.3')
            with zipfile.ZipFile(archive) as zipped:
                embedded = json.loads(zipped.read(sources.PREFIX + 'SOURCE-MANIFEST.json'))
                self.assertEqual(embedded, manifest)
                for name, sha in manifest['files'].items():
                    self.assertEqual(hashlib.sha256(zipped.read(sources.PREFIX + name)).hexdigest(), sha)
                for name in ['src-tauri/src/main.rs', 'src-tauri/native-nuked/ym3438.c',
                             'src-tauri/native-snes/snapshot/SPC_DSP.cpp', 'src-tauri/build.rs',
                             'src-tauri/Cargo.lock', 'dist/playground.js']:
                    self.assertIn(name, manifest['files'])
            before = archive.read_bytes()
            sources.prepare(root)
            self.assertEqual(archive.read_bytes(), before, 'Source ZIP must not include itself or changing timestamps')
            (root / 'src-tauri/src/main.rs').write_text('deliberate rebuild modification')
            _, changed = sources.prepare(root)
            self.assertNotEqual(changed['files']['src-tauri/src/main.rs'], manifest['files']['src-tauri/src/main.rs'])

    def test_excludes_local_state_and_build_outputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            excluded = ['.env', '.npmrc', '.git/config', 'node_modules/private-token',
                        'src-tauri/target/build/binary', 'src-tauri/gen/schemas/file',
                        'scripts/.env', 'scripts/__pycache__/cache.pyc',
                        'src-tauri/native-nuked/.DS_Store', 'audio-sidecar-bundle/node']
            for name in excluded:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('not source')
            _, manifest = sources.prepare(root)
            self.assertFalse(set(excluded) & set(manifest['files']))

    def test_rejects_source_symlink_and_missing_rebuild_material(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            link = root / 'src-tauri/native-nuked/link.c'
            try:
                link.symlink_to(root / 'README.md')
            except OSError:
                # Windows runners may not grant CreateSymbolicLink privilege.
                pass
            else:
                with self.assertRaisesRegex(ValueError, 'symbolic link'):
                    sources.prepare(root)
                link.unlink()
            (root / 'release.lock.json').unlink()
            with self.assertRaisesRegex(ValueError, 'Missing rebuild source'):
                sources.prepare(root)
