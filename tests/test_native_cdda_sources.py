import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class NativeCddaSourceTests(unittest.TestCase):
    def fixture(self, root):
        (root / 'scripts').mkdir()
        shutil.copyfile(ROOT / 'scripts/sync_native_cdda.py', root / 'scripts/sync_native_cdda.py')
        shutil.copytree(ROOT / 'src-tauri/native-cdda', root / 'src-tauri/native-cdda')

    def check(self, root, *args):
        return subprocess.run([sys.executable, str(root / 'scripts/sync_native_cdda.py'), '--check', *args],
                              cwd=root, capture_output=True, text=True)

    def test_offline_source_pin_includes_license_and_rejects_modified_c(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            valid = self.check(root)
            self.assertEqual(valid.returncode, 0, valid.stderr)
            source = root / 'src-tauri/native-cdda/cdda_pcm.c'
            source.write_bytes(source.read_bytes() + b'\n/* changed source */\n')
            changed = self.check(root)
            self.assertNotEqual(changed.returncode, 0)
            self.assertIn('source hash differs: cdda_pcm.c', changed.stderr)

    def test_missing_license_pin_and_mismatching_comparison_checkout_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.fixture(root)
            comparison = root / 'source'
            shutil.copytree(root / 'src-tauri/native-cdda', comparison / 'native/cdda')
            valid = self.check(root, '--source', str(comparison))
            self.assertEqual(valid.returncode, 0, valid.stderr)
            (comparison / 'native/cdda/LICENSE').write_text('different license')
            changed = self.check(root, '--source', str(comparison))
            self.assertNotEqual(changed.returncode, 0)
            self.assertIn('differs from comparison checkout: LICENSE', changed.stderr)
            pin = root / 'src-tauri/native-cdda/sources.json'
            manifest = json.loads(pin.read_text())
            del manifest['sha256']['LICENSE']
            pin.write_text(json.dumps(manifest))
            incomplete = self.check(root)
            self.assertNotEqual(incomplete.returncode, 0)
            self.assertIn('Invalid Native CD-DA source pin', incomplete.stderr)
