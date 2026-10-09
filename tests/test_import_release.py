import importlib.util
from pathlib import Path
import tempfile
import json
import unittest
import zipfile

spec = importlib.util.spec_from_file_location('importer', Path(__file__).resolve().parents[1] / 'scripts/import_release.py')
importer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(importer)

class ImportTests(unittest.TestCase):
    def test_new_zip_updates_without_version_and_preserves_label_for_same_zip(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / 'any-name.zip'
            def write_archive(text):
                with zipfile.ZipFile(archive, 'w') as output:
                    output.writestr('index.html', text)
                    output.writestr('playground.js', '// test')
            write_archive('first')
            first = importer.import_release(root, archive)
            first['version'] = 'named-release'
            (root / 'release.lock.json').write_text(json.dumps(first))
            self.assertEqual(importer.import_release(root, archive)['version'], 'named-release')
            write_archive('second')
            changed = importer.import_release(root, archive)
            self.assertNotEqual(changed['sha256'], first['sha256'])
            self.assertEqual(changed['version'], 'sha256-' + changed['sha256'][:12])
            self.assertEqual((root / 'dist/index.html').read_text(), 'second')
            self.assertEqual(importer.check(root), changed)

    def test_invalid_replacement_preserves_dist_and_lock(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / 'release.zip'
            with zipfile.ZipFile(archive, 'w') as output:
                output.writestr('index.html', 'valid')
                output.writestr('playground.js', '// valid')
            first = importer.import_release(root, archive)
            with zipfile.ZipFile(archive, 'w') as output:
                output.writestr('index.html', 'wrong app')
            with self.assertRaises(ValueError):
                importer.import_release(root, archive)
            self.assertEqual(importer.check(root), first)
            self.assertEqual((root / 'dist/index.html').read_text(), 'valid')

    def unpack(self, files):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / 'release.zip'
            with zipfile.ZipFile(archive, 'w') as output:
                for name, value in files.items():
                    output.writestr(name, value)
            return importer.unpack(archive, root / 'out')

    def test_preserves_bytes(self):
        files = {'index.html': b'<html>test</html>', 'playground.js': b'// test', 'generated/test.wasm': bytes(range(256))}
        self.assertEqual(self.unpack(files), {name: importer.digest(data) for name, data in files.items()})

    def test_import_excludes_macos_metadata(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive = root / 'release.zip'
            with zipfile.ZipFile(archive, 'w') as output:
                for name in ['index.html', 'playground.js', '.DS_Store', 'js/.DS_Store', '__MACOSX/._index.html']:
                    output.writestr(name, 'test')
            lock = importer.import_release(root, archive)
            self.assertEqual(set(lock['files']), {'index.html', 'playground.js'})
            self.assertEqual({p.relative_to(root / 'dist').as_posix() for p in (root / 'dist').rglob('*') if p.is_file()}, {'index.html', 'playground.js'})
            self.assertEqual(lock['sha256'], importer.digest(archive.read_bytes()))
            self.assertEqual(importer.check(root), lock)

    def test_check_ignores_metadata_in_legacy_lock_and_local_dist(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'dist').mkdir()
            (root / 'dist/index.html').write_bytes(b'original')
            lock = {'version': 'legacy', 'files': {'index.html': importer.digest(b'original'), '.DS_Store': importer.digest(b'metadata')}}
            (root / 'release.lock.json').write_text(json.dumps(lock))
            self.assertEqual(importer.check(root), lock)
            (root / 'dist/.DS_Store').write_bytes(b'changed metadata')
            self.assertEqual(importer.check(root), lock)

    def test_check_detects_modified_or_extra_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'dist').mkdir()
            target = root / 'dist/index.html'
            target.write_bytes(b'original')
            (root / 'release.lock.json').write_text(json.dumps({'version': 'test', 'files': {'index.html': importer.digest(b'original')}}))
            self.assertEqual(importer.check(root)['version'], 'test')
            target.write_bytes(b'changed')
            with self.assertRaises(ValueError):
                importer.check(root)
            target.write_bytes(b'original')
            (root / 'dist/extra.js').write_bytes(b'extra')
            with self.assertRaises(ValueError):
                importer.check(root)

    def test_rejects_traversal(self):
        for name in ['../escape', '/absolute', 'a/../../escape', 'a\\escape', 'C:/escape']:
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.unpack({name: b'bad'})

    def test_rejects_wrong_release(self):
        with self.assertRaises(ValueError):
            self.unpack({'index.html': b'not playground'})

if __name__ == '__main__':
    unittest.main()
