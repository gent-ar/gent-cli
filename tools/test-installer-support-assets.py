"""Portable fake-signing test of standalone installer dependency completeness."""
import importlib.util
from pathlib import Path
import tempfile

TOOLS = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("installer_assets", TOOLS / "stage-installer-assets.py")
assets = importlib.util.module_from_spec(spec)
spec.loader.exec_module(assets)

def main():
    with tempfile.TemporaryDirectory(prefix="gent-helper-assets-") as directory:
        output = Path(directory)
        calls = []
        def sign(command, **options):
            assert options == {"check": True}
            assert command[:4] == ["cosign", "sign-blob", "--yes", "--bundle"]
            Path(command[4]).write_bytes(b"fake signed bundle")
            calls.append(command[-1])
        assets.stage(TOOLS, output, signer=sign)
        assert len(calls) == 8
        assert (output / "gent-activate-install-support.py").read_bytes() == (TOOLS / "activate_install_support.py").read_bytes()
        assert (output / "gent-install-support.ps1").read_bytes() == (TOOLS / "install_archive_validation.ps1").read_bytes()
        for asset in assets.ASSETS.values():
            assert Path(str(output / asset) + ".sigstore.json").is_file()
        installer = (output / "gent-install.sh").read_text()
        assert 'cosign verify-blob "$temp/activate_install_support.py"' in installer
        windows = (output / "gent-install.ps1").read_text()
        assert windows.index("Assert-Signed $support $supportBundle $Version") < windows.index("    . $support")
    print("installer support assets: fake signing and verified dependency handoff passed")

if __name__ == "__main__":
    main()
