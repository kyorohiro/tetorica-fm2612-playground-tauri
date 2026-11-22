# SPDX-License-Identifier: BSD-3-Clause
"""ZIP import/version integration checks using temporary application fixtures."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest import mock
import zipfile


REPOSITORY = Path(__file__).resolve().parents[1]
ENTRY = "playground.js"
METADATA = ('package.json', 'package-lock.json', 'src-tauri/Cargo.toml',
            'src-tauri/tauri.conf.json', 'src-tauri/Cargo.lock')


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ImportVersionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='import version fixture ')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / 'scripts').mkdir()
        (self.root / 'src-tauri').mkdir()
        for name in ('import_release.py', 'app_version.py'):
            shutil.copyfile(REPOSITORY / 'scripts' / name, self.root / 'scripts' / name)
        self.importer = load_module('temporary_version_importer', self.root / 'scripts/import_release.py')
        self.native_fixtures = (load_module('version_release_fixtures', REPOSITORY / 'tests/test_import_release.py')
                                if ENTRY == 'synth-snes/synth.js' else None)
        for name, value in {
            'package.json': {'name': 'fixture', 'version': '1.2.3'},
            'package-lock.json': {
                'name': 'fixture', 'version': '0.8.0', 'lockfileVersion': 3,
                'packages': {'': {'name': 'fixture', 'version': '0.8.0'},
                             'node_modules/example': {'version': '4.5.6'}},
            },
            'src-tauri/tauri.conf.json': {'productName': 'Fixture', 'version': '1.2.3'},
        }.items():
            (self.root / name).write_text(json.dumps(value, indent=2) + '\n')
        (self.root / 'src-tauri/Cargo.toml').write_text(
            '[package]\nname = "fixture"\nversion = "1.2.3"\nedition = "2021"\n'
            '\n[dependencies]\nexample = "4.5.6"\n')
        (self.root / 'src-tauri/Cargo.lock').write_text(
            'version = 4\n\n[[package]]\nname = "fixture"\nversion = "1.2.3"\ndependencies = ["example"]\n\n'
            '[[package]]\nname = "example"\nversion = "4.5.6"\n'
            'source = "registry+https://example.invalid/index"\nchecksum = "abc123"\n')
        self.old_archive = self.root / 'old-source.zip'
        self.archive = self.root / 'source-name-independent-of-app-version.zip'
        self.write_zip(self.old_archive, b'old sound')
        self.write_zip(self.archive, b'new sound')
        self.import_quiet(self.root, self.old_archive)

    def write_zip(self, archive, text):
        if self.native_fixtures:
            files = self.native_fixtures.release_files(text)
            browser = json.loads(files['sources/sdsp-synth-browser.json'])
            browser['version'] = '8.7.6-web'
            files['sources/sdsp-synth-browser.json'] = json.dumps(browser).encode()
        else:
            files = {'index.html': text, ENTRY: b'// readable source fixture'}
        with zipfile.ZipFile(archive, 'w') as output:
            for name, data in files.items():
                output.writestr(name, data)

    def import_quiet(self, root, archive, **options):
        with contextlib.redirect_stdout(io.StringIO()):
            return self.importer.import_release(root, archive, **options)

    def snapshot(self):
        names = [*METADATA, 'release.lock.json', 'local-changes.json']
        result = {name: (self.root / name).read_bytes()
                  for name in names if (self.root / name).is_file()}
        result.update({'dist/' + path.relative_to(self.root / 'dist').as_posix(): path.read_bytes()
                       for path in (self.root / 'dist').rglob('*') if path.is_file()})
        return result

    def metadata(self):
        return {name: (self.root / name).read_bytes() for name in METADATA}

    def cli(self, *arguments):
        return subprocess.run(
            [sys.executable, str(self.root / 'scripts/import_release.py'), *map(str, arguments)],
            cwd=self.root, capture_output=True, text=True, check=False)

    def assert_versions(self, expected):
        package = json.loads((self.root / 'package.json').read_text())
        npm_lock = json.loads((self.root / 'package-lock.json').read_text())
        cargo = tomllib.loads((self.root / 'src-tauri/Cargo.toml').read_text())
        config = json.loads((self.root / 'src-tauri/tauri.conf.json').read_text())
        lock = tomllib.loads((self.root / 'src-tauri/Cargo.lock').read_text())
        own = [p for p in lock['package'] if p['name'] == 'fixture' and 'source' not in p]
        self.assertEqual(
            [package['version'], npm_lock['version'], npm_lock['packages']['']['version'],
             cargo['package']['version'], config['version'], own[0]['version']],
            [expected] * 6)
        self.assertEqual(npm_lock['packages']['node_modules/example']['version'], '4.5.6')
        self.assertEqual([p for p in lock['package'] if p['name'] == 'example'], [{
            'name': 'example', 'version': '4.5.6',
            'source': 'registry+https://example.invalid/index', 'checksum': 'abc123',
        }])

    def test_cli_syncs_all_app_versions_without_relabeling_source_receipt(self):
        result = self.cli('--version', '2.3.4', self.archive)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assert_versions('2.3.4')
        receipt = self.importer.check(self.root)
        sha = self.importer.digest(self.archive.read_bytes())
        self.assertEqual(receipt['archive'], self.archive.name)
        self.assertEqual(receipt['sha256'], sha)
        self.assertEqual(receipt['version'], '8.7.6-web' if self.native_fixtures else 'sha256-' + sha[:12])
        self.assertNotEqual(receipt['version'], '2.3.4')
        for name, digest in receipt['files'].items():
            self.assertEqual(self.importer.digest((self.root / 'dist' / name).read_bytes()), digest)

    def test_invalid_version_never_changes_metadata_or_current_sound(self):
        before = self.snapshot()
        for version in ('01.2.3', '1.2', 'v1.2.3', '../1.2.3', '1.2.3-beta'):
            with self.subTest(version=version), self.assertRaises(ValueError):
                self.import_quiet(self.root, self.archive, version=version)
            self.assertEqual(self.snapshot(), before)

    def test_malformed_metadata_never_changes_metadata_or_current_sound(self):
        path = self.root / 'package-lock.json'
        original = path.read_bytes()
        for text in ('{broken', '{"version":"1.2.3","packages":{}}'):
            path.write_text(text)
            before = self.snapshot()
            with self.subTest(text=text), self.assertRaises(ValueError):
                self.import_quiet(self.root, self.archive, version='2.3.4')
            self.assertEqual(self.snapshot(), before)
            path.write_bytes(original)

    def test_bad_zip_preserves_existing_metadata_dist_and_receipt(self):
        invalid = self.root / 'invalid.zip'
        with zipfile.ZipFile(invalid, 'w') as output:
            output.writestr('index.html', 'missing application module')
        before = self.snapshot()
        with self.assertRaises(ValueError):
            self.import_quiet(self.root, invalid, version='2.3.4')
        self.assertEqual(self.snapshot(), before)
        invalid.write_bytes(b'not a ZIP')
        with self.assertRaises(zipfile.BadZipFile):
            self.import_quiet(self.root, invalid, version='2.3.4')
        self.assertEqual(self.snapshot(), before)

    def test_legacy_import_keeps_metadata_and_needs_no_app_metadata_files(self):
        before = self.metadata()
        self.assertIsNone(self.importer.prepare_app_version(self.root, None))
        self.import_quiet(self.root, self.archive)
        self.assertEqual(self.metadata(), before)
        standalone = self.root / 'legacy-without-app-metadata'
        standalone.mkdir()
        lock = self.import_quiet(standalone, self.archive)
        self.assertEqual(self.importer.check(standalone), lock)
        self.assertFalse(any((standalone / name).exists() for name in METADATA))

    def test_apply_failure_restores_dist_receipt_local_manifest_and_app_metadata(self):
        local = self.root / 'local-changes.json'
        if self.native_fixtures:
            (self.root / 'dist/index.html').write_bytes(b'local sound edit')
            with contextlib.redirect_stdout(io.StringIO()):
                self.importer.record_local_changes(self.root)
        else:
            local.write_bytes(b'{"fixture":"existing local record"}\n')
        before = self.snapshot()
        update = self.importer.prepare_app_version(self.root, '2.3.4')
        replace = os.replace
        calls = 0

        def fail_second(source, destination):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise OSError('fixture metadata replacement failed')
            return replace(source, destination)

        with mock.patch.object(self.importer, 'prepare_app_version', return_value=update), \
             mock.patch.object(os, 'replace', side_effect=fail_second):
            with self.assertRaisesRegex(OSError, 'fixture metadata replacement failed'):
                self.import_quiet(self.root, self.archive, version='2.3.4')
        self.assertGreater(calls, 2)
        self.assertEqual(self.snapshot(), before)
        self.assertTrue(local.is_file())
        self.assertFalse(list(self.root.glob('.import-*')))
        self.assertFalse(list(self.root.glob('.app-version-*')))

    def test_cli_rejects_version_with_check_and_record_actions(self):
        before = self.snapshot()
        actions = ['--check']
        if self.native_fixtures:
            actions.append('--record-local-changes')
        for action in actions:
            result = self.cli(action, '--version', '2.3.4')
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn('--version', result.stderr)
            self.assertEqual(self.snapshot(), before)

    def test_invalid_cli_version_fails_without_any_import_changes(self):
        before = self.snapshot()
        result = self.cli(self.archive, '--version', 'not-a-version')
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('numeric X.Y.Z', result.stderr)
        self.assertEqual(self.snapshot(), before)


if __name__ == '__main__':
    unittest.main()
