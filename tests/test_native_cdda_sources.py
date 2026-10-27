"""Fixed SDK application-audio guard; no old source mirror or parent needed."""
from pathlib import Path
import shutil
import subprocess
import sys
import unittest

import test_soundchip_sdk as fixture_module

ROOT = Path(__file__).resolve().parents[1]
COMPONENT = 'cdda'


class NativeApplicationAudioSourceTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixture_module.SoundchipSdkTests(methodName='runTest')
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.pin = self.fixture.install(application_audio=True)
        scripts = self.root / 'scripts'
        scripts.mkdir()
        for name in ('import_soundchip_sdk.py', 'verify_sdk_application_audio.py', f'sync_native_{COMPONENT}.py'):
            shutil.copyfile(ROOT / 'scripts' / name, scripts / name)

    def command(self, *args):
        return subprocess.run([sys.executable, str(self.root / 'scripts' / f'sync_native_{COMPONENT}.py'), *args],
                              cwd=self.root, capture_output=True, text=True)

    def test_fixed_sdk_is_sufficient_and_tampered_compiled_source_is_rejected(self):
        self.assertFalse((self.root / 'src-tauri' / f'native-{COMPONENT}').exists())
        self.fixture.work.joinpath('sdk.zip').unlink()
        valid = self.command('--check')
        self.assertEqual(valid.returncode, 0, valid.stderr)
        self.assertIn('verified fixed release', valid.stdout)
        source = self.root / fixture_module.sdk.SNAPSHOT / f'vendor/tetorica-{COMPONENT}/{COMPONENT}_pcm.c'
        source.write_bytes(source.read_bytes() + b'\n/* changed */\n')
        changed = self.command('--check')
        self.assertNotEqual(changed.returncode, 0)
        self.assertIn('hash/size mismatch', changed.stderr)

    def test_source_override_is_rejected_without_writes_and_missing_license_is_rejected(self):
        receipt = self.root / fixture_module.sdk.RECEIPT
        original = receipt.read_bytes()
        rejected = self.command('--source', str(self.fixture.work))
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn('unrecognized arguments', rejected.stderr)
        self.assertEqual(receipt.read_bytes(), original)
        missing = self.root / fixture_module.sdk.SNAPSHOT / f'vendor/tetorica-{COMPONENT}/LICENSE'
        missing.unlink()
        self.assertNotEqual(self.command('--check').returncode, 0)
