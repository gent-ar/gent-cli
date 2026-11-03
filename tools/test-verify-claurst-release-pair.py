import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('pair', Path(__file__).with_name('verify-claurst-release-pair.py'))
pair = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pair)


class ClaurstPair(unittest.TestCase):
    def test_pinned_payload_must_match_and_candidate_may_add_model_runtime(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            official, sandbox = root / 'official', root / 'sandbox'
            for runtime in (official, sandbox):
                (runtime / 'llama').mkdir(parents=True)
                (runtime / 'claurst.exe').write_bytes(b'pinned-sdk')
                (runtime / 'llama/llama-server.exe').write_bytes(b'pinned-model-server')
            (sandbox / 'model-server').mkdir()
            (sandbox / 'model-server/model-runtime.json').write_bytes(b'model-binding')
            pair.verify(official, sandbox)
            extra = sandbox / 'llama/unreviewed.dll'
            extra.write_bytes(b'not-in-pinned-runtime')
            with self.assertRaisesRegex(ValueError, 'llama file set differs'):
                pair.verify(official, sandbox)
            extra.unlink()
            (sandbox / 'claurst.exe').write_bytes(b'different-sdk')
            with self.assertRaisesRegex(ValueError, 'differs from the pinned release'):
                pair.verify(official, sandbox)

    def test_missing_official_file_in_sandbox_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            official, sandbox = root / 'official', root / 'sandbox'
            official.mkdir(); sandbox.mkdir()
            (official / 'claurst.exe').write_bytes(b'pinned-sdk')
            with self.assertRaises(ValueError):
                pair.verify(official, sandbox)


if __name__ == '__main__':
    unittest.main()
