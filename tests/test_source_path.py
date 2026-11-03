import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('parent_source_path', ROOT / 'scripts/source_path.py')
paths = importlib.util.module_from_spec(spec)
spec.loader.exec_module(paths)


class SourcePathTests(unittest.TestCase):
    def test_legacy_and_moved_checkouts_read_the_same_sources(self):
        with tempfile.TemporaryDirectory(prefix='source paths ') as temporary:
            root = Path(temporary)
            for modern in (False, True):
                checkout = root / ('modern' if modern else 'legacy')
                checkout.mkdir()
                if modern:
                    (checkout / 'tetorica_src').mkdir()
                for original, moved in paths.MOVED_ROOTS.items():
                    expected = (checkout / 'tetorica_src' / moved if modern else checkout / original) / 'file'
                    expected.parent.mkdir(parents=True, exist_ok=True)
                    expected.write_bytes(original.encode())
                    self.assertEqual(paths.source_path(checkout, original + '/file'), expected)
                    self.assertEqual(paths.source_path(checkout, original + '/file').read_bytes(), original.encode())
                for name in ('src/upstream.cpp', 'third_party/vendor/source.cpp', 'docs/generated/core.wasm'):
                    expected = checkout / name
                    expected.parent.mkdir(parents=True, exist_ok=True)
                    expected.write_bytes(b'unmoved')
                    self.assertEqual(paths.source_path(checkout, name).read_bytes(), b'unmoved')

    def test_missing_modern_sources_do_not_fall_back_to_obsolete_copies(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'tetorica_src').mkdir()
            (root / 'web').mkdir()
            (root / 'web/obsolete.js').write_bytes(b'obsolete')
            with self.assertRaises(FileNotFoundError):
                paths.source_path(root, 'web/obsolete.js').read_bytes()

    def test_noise_verifier_accepts_both_parent_layouts_without_changing_pins(self):
        with tempfile.TemporaryDirectory(prefix='source verifier ') as temporary:
            root = Path(temporary)
            scripts = root / 'desktop/scripts'
            scripts.mkdir(parents=True)
            for name in ('sync_native_genesis_noise.py', 'source_path.py'):
                shutil.copyfile(ROOT / 'scripts' / name, scripts / name)
            snapshot = root / 'desktop/src-tauri/native-noise'
            shutil.copytree(ROOT / 'src-tauri/native-noise', snapshot)
            before = {p.relative_to(snapshot): p.read_bytes() for p in snapshot.rglob('*') if p.is_file()}
            raw = (snapshot / 'snapshot/native/audio_effect/noise.c').read_bytes()
            command = [sys.executable, str(scripts / 'sync_native_genesis_noise.py'), '--check']
            for modern in (False, True):
                parent = root / ('modern parent' if modern else 'legacy parent')
                target = parent / ('tetorica_src/native/audio_effect/noise.c' if modern else 'native/audio_effect/noise.c')
                target.parent.mkdir(parents=True)
                target.write_bytes(raw)
                result = subprocess.run([*command, '--source', str(parent)], capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                target.unlink()
                missing = subprocess.run([*command, '--source', str(parent)], capture_output=True, text=True)
                self.assertNotEqual(missing.returncode, 0)
                self.assertIn('FileNotFoundError', missing.stderr)
            self.assertEqual(before, {p.relative_to(snapshot): p.read_bytes() for p in snapshot.rglob('*') if p.is_file()})

    def test_genesis_verifier_resolves_moved_wrappers_and_preserved_upstream_sources(self):
        with tempfile.TemporaryDirectory(prefix='genesis source verifier ') as temporary:
            root = Path(temporary)
            scripts = root / 'desktop/scripts'
            scripts.mkdir(parents=True)
            for name in ('sync_native_genesis.py', 'source_path.py'):
                shutil.copyfile(ROOT / 'scripts' / name, scripts / name)
            snapshot = root / 'desktop/src-tauri/native-genesis'
            shutil.copytree(ROOT / 'src-tauri/native-genesis', snapshot)
            manifest_bytes = (snapshot / 'sources.json').read_bytes()
            keys = json.loads(manifest_bytes)['sha256']
            for modern in (False, True):
                parent = root / ('modern' if modern else 'legacy')
                parent.mkdir()
                if modern:
                    (parent / 'tetorica_src').mkdir()
                for key in keys:
                    name = 'LICENSE.segapsg' if key == 'third_party/tetorica_sega_psg/LICENSE' else Path(key).name
                    target = paths.source_path(parent, key)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes((snapshot / name).read_bytes())
                result = subprocess.run([sys.executable, str(scripts / 'sync_native_genesis.py'),
                                         '--check', '--source', str(parent)], capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((snapshot / 'sources.json').read_bytes(), manifest_bytes)
