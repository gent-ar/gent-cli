from pathlib import Path
import hashlib
import json
import tempfile
import unittest
from unittest import mock

from windows_sandbox_package import MAX_PROVIDER_MANIFEST_BYTES, read_provider_manifest

ACTUAL=Path(__file__).resolve().parent.parent/'crates/gentd/tests/fixtures/windows-unified-provider-resources.json'

class Bounds(unittest.TestCase):
    def setUp(self):
        self.directory=tempfile.TemporaryDirectory(prefix='gent-provider-bound-')
        self.addCleanup(self.directory.cleanup)
        self.path=Path(self.directory.name)/'provider-resources.json'

    def test_actual_three_providers_and_exact_512k_boundary(self):
        data=ACTUAL.read_bytes()
        self.assertEqual(len(data),310805)
        self.path.write_bytes(data)
        self.assertEqual([p['provider'] for p in read_provider_manifest(self.path,hashlib.sha256(data).hexdigest())['providers']],['codex','claude','claurst'])
        self.path.write_bytes(data+b' '*(MAX_PROVIDER_MANIFEST_BYTES-len(data)))
        self.assertEqual(3,len(read_provider_manifest(self.path)['providers']))

    def test_512k_plus_one_rejected_before_json_decoder(self):
        self.assertEqual(MAX_PROVIDER_MANIFEST_BYTES,524288)
        self.path.write_bytes(b'['*(MAX_PROVIDER_MANIFEST_BYTES+1))
        with mock.patch('windows_sandbox_package.json.loads',side_effect=AssertionError('decoder must not run')):
            with self.assertRaisesRegex(ValueError,'manifest exceeds its bound'):
                read_provider_manifest(self.path)

    def test_exact_pin_and_duplicate_json_rejection_remain(self):
        self.path.write_bytes(ACTUAL.read_bytes())
        with self.assertRaisesRegex(ValueError,'does not match descriptor'):
            read_provider_manifest(self.path,'0'*64)
        self.path.write_text('{"version":1,"version":1}')
        with self.assertRaisesRegex(ValueError,'duplicate'):
            read_provider_manifest(self.path)

if __name__=='__main__':unittest.main()
