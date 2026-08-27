#!/usr/bin/env python3
import hashlib
import importlib.util
import tarfile
import tempfile
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location("stage", ROOT / "tools" / "stage-claurst-runtime.py")
stage = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(stage)


def sha(content):
    return hashlib.sha256(content).hexdigest()


def main():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source = root / "source"
        source.mkdir()
        output = root / "output"
        tar_path = source / "claurst.tar.gz"
        with tarfile.open(tar_path, "w:gz") as archive:
            member = tarfile.TarInfo("claurst")
            content = b"claurst"
            member.size = len(content)
            import io
            archive.addfile(member, io.BytesIO(content))
        zip_path = source / "llama.zip"
        with zipfile.ZipFile(zip_path, "w") as archive:
            archive.writestr("bin/llama-server.exe", b"llama")
        claurst_digest = sha(tar_path.read_bytes())
        original = stage.ARTIFACTS
        stage.ARTIFACTS = {"fixture": {"claurst": ("https://fixture.invalid/claurst.tar.gz", claurst_digest, "claurst"), "llama": ("https://fixture.invalid/llama.zip", sha(zip_path.read_bytes()), "llama-server.exe")}}
        import sys
        previous = sys.argv
        sys.argv = ["stage", "--target", "fixture", "--out-dir", str(output), "--source-dir", str(source)]
        stage.main()
        sys.argv = previous
        stage.ARTIFACTS = original
        assert (output / "claurst").read_bytes() == b"claurst"
        assert (output / "llama/llama-server.exe").read_bytes() == b"llama"
        assert (output / "claurst").stat().st_mode & 0o111
        assert (output / "llama/llama-server.exe").stat().st_mode & 0o111
        tampered = source / "claurst.tar.gz"
        tampered.write_bytes(b"bad")
        try:
            stage.source_for(source, "https://fixture.invalid/claurst.tar.gz", claurst_digest)
        except ValueError:
            pass
        else:
            raise AssertionError("tampered source was accepted")
    print("Claurst runtime staging checks passed")


if __name__ == "__main__":
    main()
