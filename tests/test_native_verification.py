import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile

import test_native_sources as fixtures

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('native_verification', ROOT / 'scripts/check_native_sources.py')
verification = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verification)


class NativeVerificationTests(unittest.TestCase):
    def prepared_fixture(self, root):
        fixtures.NativeSourceTests().fixture(root)
        shutil.copyfile(ROOT / 'scripts/prepare_native_sources.py', root / 'scripts/prepare_native_sources.py')
        archive, manifest = fixtures.sources.prepare(root)
        return archive, manifest

    def test_real_bundle_verifier_accepts_matching_sources_and_rejects_stale_checkout(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _, manifest = self.prepared_fixture(root)
            result = verification.verify_bundle(root)
            self.assertEqual(result['status'], 'passed')
            self.assertEqual(result['files'], len(manifest['files']))
            (root / 'src-tauri/src/main.rs').write_text('changed after source ZIP preparation')
            with self.assertRaisesRegex(ValueError, 'stale relative'):
                verification.verify_bundle(root)

    def test_bundle_verifier_rejects_corrupt_zip_and_mismatched_external_manifest(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive, _ = self.prepared_fixture(root)
            with zipfile.ZipFile(archive) as source:
                entries = [(info, source.read(info)) for info in source.infolist()]
            with zipfile.ZipFile(archive, 'w') as output:
                for info, data in entries:
                    output.writestr(info, b'tampered C++ bytes' if info.filename.endswith('/src/main.rs') else data)
            with self.assertRaisesRegex(ValueError, 'content hash mismatch'):
                verification.verify_bundle(root)
            self.prepared_fixture(root)
            manifest_path = root / 'native-source-bundle/SOURCE-MANIFEST.json'
            manifest = json.loads(manifest_path.read_text())
            manifest['files']['src-tauri/src/main.rs'] = '0' * 64
            manifest_path.write_text(json.dumps(manifest))
            with self.assertRaisesRegex(ValueError, 'Embedded and external'):
                verification.verify_bundle(root)

    def test_older_generated_snapshots_verify_without_parent_and_detect_input_or_output_tampering(self):
        # Only committed scripts/snapshots exist in this isolated desktop tree.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'isolated-desktop'
            scripts = root / 'scripts'
            scripts.mkdir(parents=True)
            for suffix, directory, target in [
                    ('fx', 'native-fx', 'gain.inc'),
                    ('noise', 'native-noise', 'noise.inc'),
                    ('livefx', 'native-livefx', 'custom_fx.inc')]:
                script = 'sync_native_genesis_' + suffix + '.py'
                shutil.copyfile(ROOT / 'scripts' / script, scripts / script)
                destination = root / 'src-tauri' / directory
                shutil.copytree(ROOT / 'src-tauri' / directory, destination)
                command = [sys.executable, str(scripts / script), '--check']
                initial = subprocess.run(command, cwd=root, capture_output=True)
                self.assertEqual(initial.returncode, 0, initial.stderr.decode())
                output = destination / target
                original = output.read_bytes()
                output.write_bytes(original + b'\n// unpinned change\n')
                changed = subprocess.run(command, cwd=root, capture_output=True)
                self.assertNotEqual(changed.returncode, 0)
                output.write_bytes(original)
                pinned = next((destination / 'snapshot').rglob('*.c' if suffix != 'livefx' else '*.js'))
                pinned.write_bytes(pinned.read_bytes() + b'\n// modified original\n')
                changed_input = subprocess.run(command, cwd=root, capture_output=True)
                self.assertNotEqual(changed_input.returncode, 0)
