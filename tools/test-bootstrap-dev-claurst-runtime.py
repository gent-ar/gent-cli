import importlib.util
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location(
    "bootstrap", ROOT / "tools/bootstrap-dev-claurst-runtime.py"
)
bootstrap = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bootstrap)


def main():
    with tempfile.TemporaryDirectory() as directory:
        output = Path(directory) / "runtime"
        calls = []

        def run(command, **kwargs):
            calls.append((command, kwargs))
            if command == ["rustc", "-vV"]:
                return subprocess.CompletedProcess(command, 0, "host: aarch64-apple-darwin\n", "")
            return subprocess.CompletedProcess(command, 0)

        previous_run = bootstrap.subprocess.run
        previous_argv = sys.argv
        try:
            bootstrap.subprocess.run = run
            sys.argv = [
                "bootstrap",
                "--out-dir",
                str(output),
                "--source-dir",
                "cache",
                "--claurst-source-dir",
                "upstream",
            ]
            bootstrap.main()
        finally:
            bootstrap.subprocess.run = previous_run
            sys.argv = previous_argv
        assert calls[0][0] == ["rustc", "-vV"]
        assert calls[1][0] == [
            sys.executable,
            str(ROOT / "tools/stage-claurst-runtime.py"),
            "--target",
            "aarch64-apple-darwin",
            "--out-dir",
            str(output),
            "--profile",
            "debug",
            "--source-dir",
            "cache",
            "--claurst-source-dir",
            "upstream",
        ]
        calls.clear()
        try:
            bootstrap.subprocess.run = run
            sys.argv = ["bootstrap", "--profile", "release"]
            bootstrap.main()
        finally:
            bootstrap.subprocess.run = previous_run
            sys.argv = previous_argv
        assert calls[1][0][5] == str(ROOT / "target/release/runtime/claurst")
    print("development Claurst bootstrap checks passed")


if __name__ == "__main__":
    main()
