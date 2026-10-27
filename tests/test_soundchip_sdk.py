import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import warnings
import zipfile

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts/import_soundchip_sdk.py'
spec = importlib.util.spec_from_file_location('soundchip_sdk', SCRIPT)
sdk = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sdk)


class SoundchipSdkTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name)
        self.root = self.work / 'standalone app'
        self.root.mkdir()

    def archive(self, *, version='0.1.0', alter=None):
        payload = {name: ('pinned SDK source: ' + name).encode() for name in sdk.REQUIRED | sdk.SEPARATED_SEGA_PSG}
        payload['sdk.json'] = json.dumps({'name': sdk.NAME, 'version': version}).encode()
        manifest = {'schemaVersion': 1, 'kind': 'source', 'name': sdk.NAME, 'version': version,
                    'files': {name: {'sha256': sdk.digest(data), 'size': len(data)}
                              for name, data in payload.items()}}
        payload[sdk.MANIFEST] = json.dumps(manifest).encode()
        prefix = sdk.NAME + '-' + version + '/'
        entries = [(prefix + name, data, stat.S_IFREG | 0o644) for name, data in payload.items()]
        if alter:
            entries = alter(entries)
        path = self.work / 'sdk.zip'
        with warnings.catch_warnings():
            warnings.simplefilter('ignore', UserWarning)
            with zipfile.ZipFile(path, 'w') as archive:
                for name, data, mode in entries:
                    info = zipfile.ZipInfo(name)
                    info.create_system = 3
                    info.external_attr = mode << 16
                    archive.writestr(info, data)
        return path, sdk.digest(path.read_bytes())

    def install(self, **options):
        path, checksum = self.archive(**options)
        return sdk.import_archive(path, checksum, 'a' * 40, self.root)

    def test_unpublished_candidate_records_base_commit_and_changed_paths(self):
        path, checksum = self.archive()
        pin = sdk.import_archive(path, checksum, 'a' * 40, self.root,
                                 local_changed_paths=['sdk.json', 'LICENSE'])
        self.assertEqual(pin['localCandidate'], {'baseCommit': 'a' * 40,
                         'changedPaths': ['LICENSE', 'sdk.json']})
        self.assertEqual(sdk.check_installed(self.root), pin)

    def test_unpublished_candidate_rejects_unknown_or_unsafe_changed_paths(self):
        path, checksum = self.archive()
        for paths in [['../sdk.json'], ['not-in-release.cpp']]:
            with self.subTest(paths=paths):
                with self.assertRaises(ValueError):
                    sdk.import_archive(path, checksum, 'a' * 40, self.root,
                                       local_changed_paths=paths)

    def test_local_archive_import_check_idempotence_and_explicit_update(self):
        initial = self.install()
        self.assertEqual(sdk.check_installed(self.root), initial)
        data = (self.root / sdk.SNAPSHOT / 'vendor/ymfm/ymfm_opn.cpp').read_bytes()
        self.assertEqual(self.install(), initial)
        updated = self.install(version='0.2.0')
        self.assertNotEqual(updated['archiveSha256'], initial['archiveSha256'])
        self.assertEqual(updated['version'], '0.2.0')
        self.assertEqual((self.root / sdk.SNAPSHOT / 'vendor/ymfm/ymfm_opn.cpp').read_bytes(), data)
        self.assertEqual(sdk.check_installed(self.root), updated)

    def test_cli_checks_copy_without_original_archive_or_sdk_checkout(self):
        self.install()
        self.work.joinpath('sdk.zip').unlink()
        script = self.root / 'scripts/import_soundchip_sdk.py'
        script.parent.mkdir()
        shutil.copyfile(SCRIPT, script)
        result = subprocess.run([sys.executable, str(script), '--check'], cwd=self.root,
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('verified offline', result.stdout)

    def test_archive_checksum_failure_does_not_install(self):
        path, _ = self.archive()
        with self.assertRaisesRegex(ValueError, 'SHA-256 mismatch'):
            sdk.import_archive(path, '0' * 64, 'a' * 40, self.root)
        self.assertFalse((self.root / sdk.SNAPSHOT).exists())

    def test_imports_exact_hashed_bytes_if_archive_path_changes_after_read(self):
        path, checksum = self.archive(version='0.1.0')
        original = path.read_bytes()
        self.archive(version='0.2.0')
        replacement = path.read_bytes()
        path.write_bytes(original)
        read = Path.read_bytes

        def replace_after_read(current):
            data = read(current)
            if current == path:
                current.write_bytes(replacement)
            return data

        with patch.object(Path, 'read_bytes', replace_after_read):
            pin = sdk.import_archive(path, checksum, 'a' * 40, self.root)
        self.assertEqual(pin['version'], '0.1.0')
        self.assertEqual(pin['archiveSha256'], checksum)
        self.assertEqual(sdk.check_installed(self.root), pin)

    def test_content_and_manifest_corruption_do_not_replace_current_sources(self):
        initial = self.install()
        for change in ('bytes', 'missing', 'duplicate', 'extra', 'symlink', 'escape', 'case'):
            with self.subTest(change=change):
                def alter(entries):
                    name, data, mode = entries[0]
                    if change == 'bytes':
                        return [(name, b'changed bytes', mode), *entries[1:]]
                    if change == 'missing':
                        return entries[1:]
                    if change == 'duplicate':
                        return [*entries, entries[0]]
                    if change == 'symlink':
                        return [(name, data, stat.S_IFLNK | 0o777), *entries[1:]]
                    if change == 'escape':
                        return [*entries, ('../escape.cpp', b'outside', mode)]
                    if change == 'case':
                        return [*entries, (name.upper(), data, mode)]
                    return [*entries, ('tetorica-soundchip-native-0.1.0/extra.cpp', b'extra', mode)]
                path, checksum = self.archive(alter=alter)
                with self.assertRaises(ValueError):
                    sdk.import_archive(path, checksum, 'a' * 40, self.root)
                self.assertEqual(sdk.check_installed(self.root), initial)
                self.assertFalse((self.root / 'escape.cpp').exists())

    def test_local_edits_or_added_files_are_not_silently_overwritten(self):
        self.install()
        path = self.root / sdk.SNAPSHOT / 'vendor/ymfm/ymfm_opn.cpp'
        original = path.read_bytes()
        path.write_bytes(b'downstream edit')
        with self.assertRaisesRegex(ValueError, 'hash/size mismatch'):
            self.install(version='0.2.0')
        self.assertEqual(path.read_bytes(), b'downstream edit')
        path.write_bytes(original)
        extra = self.root / sdk.SNAPSHOT / 'unrecorded.cpp'
        extra.write_bytes(b'unrecorded')
        with self.assertRaises(ValueError):
            sdk.check_installed(self.root)

    def test_manifest_or_receipt_changes_are_detected(self):
        self.install()
        path = self.root / sdk.SNAPSHOT / sdk.MANIFEST
        original = path.read_bytes()
        path.write_bytes(original + b'\n')
        with self.assertRaisesRegex(ValueError, 'manifest differs'):
            sdk.check_installed(self.root)
        path.write_bytes(original)
        receipt = self.root / sdk.RECEIPT
        pin = json.loads(receipt.read_bytes())
        pin['version'] = 'wrong-version'
        receipt.write_text(json.dumps(pin))
        with self.assertRaisesRegex(ValueError, 'pin version/file count'):
            sdk.check_installed(self.root)

    def test_rejects_symbolic_links_in_snapshot_or_vendor_parent(self):
        self.install()
        snapshot = self.root / sdk.SNAPSHOT
        link = snapshot / 'linked-source'
        try:
            link.symlink_to(self.root / sdk.RECEIPT)
        except OSError:
            self.skipTest('Symbolic links are unavailable on this runner')
        with self.assertRaisesRegex(ValueError, 'symbolic link'):
            sdk.check_installed(self.root)
        link.unlink()
        vendor = snapshot.parent
        moved = self.root / 'moved vendor'
        vendor.rename(moved)
        vendor.symlink_to(moved, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'parent must not be a symbolic link'):
            sdk.check_installed(self.root)
        with self.assertRaises(ValueError):
            self.install()
