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
            self.unpack({'index.html': b'not analyzer'})

if __name__ == '__main__':
    unittest.main()
