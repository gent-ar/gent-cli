#!/usr/bin/env sh
# Install or update a signed Gent release. Run only from a trusted source.
set -eu
umask 077

repo='gent-ar/gent-cli'
install_root=${GENT_INSTALL_DIR:-"$HOME/.local"}
version=${GENT_VERSION:-}
force=0
expected_sha256=''
idle_data_dir=''

usage() {
  printf '%s\n' 'usage: install.sh [--version vX.Y.Z] [--install-dir DIR] [--expected-sha256 DIGEST] [--idle-data-dir DIR] [--force]'
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version) version=${2:?missing version}; shift 2 ;;
    --install-dir) install_root=${2:?missing directory}; shift 2 ;;
    --expected-sha256) expected_sha256=${2:?missing digest}; shift 2 ;;
    --idle-data-dir) idle_data_dir=${2:?missing directory}; shift 2 ;;
    --force) force=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
done

command -v curl >/dev/null || { printf '%s\n' 'curl is required' >&2; exit 1; }
command -v cosign >/dev/null || { printf '%s\n' 'cosign is required for signed Gent installs' >&2; exit 1; }
command -v python3 >/dev/null || { printf '%s\n' 'python3 is required for manifest verification' >&2; exit 1; }
command -v tar >/dev/null || { printf '%s\n' 'tar is required' >&2; exit 1; }

os=$(uname -s)
arch=$(uname -m)
case "$os/$arch" in
  Darwin/arm64) target='aarch64-apple-darwin' ;;
  Darwin/x86_64) target='x86_64-apple-darwin' ;;
  Linux/x86_64) target='x86_64-unknown-linux-gnu' ;;
  *) printf 'unsupported platform: %s/%s\n' "$os" "$arch" >&2; exit 1 ;;
esac

if [ -z "$version" ]; then
  version=$(curl --fail --silent --show-error "https://api.github.com/repos/$repo/releases/latest" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["tag_name"])')
fi
case "$version" in v[0-9]*.[0-9]*.[0-9]*) ;; *) printf 'invalid release version: %s\n' "$version" >&2; exit 1 ;; esac
case "$expected_sha256" in ''|*[!0-9a-f]*) printf '%s\n' 'expected digest must be lowercase hexadecimal' >&2; exit 1 ;; esac
[ -z "$expected_sha256" ] || [ "${#expected_sha256}" -eq 64 ] || { printf '%s\n' 'expected digest must contain 64 hexadecimal characters' >&2; exit 1; }

name="gent-$version-$target.tar.gz"
release_base=${GENT_RELEASE_BASE_URL:-"https://github.com/$repo/releases/download/$version"}
release_base=${release_base%/}
base="$release_base/$name"
temp=$(mktemp -d "${TMPDIR:-/tmp}/gent-install.XXXXXX")
cleanup() { rm -rf "$temp"; }
trap cleanup EXIT HUP INT TERM

download() { curl --fail --location --silent --show-error --output "$temp/$2" "$1"; }
download "$base" "$name"
download "$base.sha256" "$name.sha256"
download "$base.manifest.json" "$name.manifest.json"
download "$base.sigstore.json" "$name.sigstore.json"
download "$base.manifest.json.sigstore.json" "$name.manifest.json.sigstore.json"
helper_base="$release_base/gent-activate-install.py"
download "$helper_base" 'gent-activate-install.py'
download "$helper_base.sigstore.json" 'gent-activate-install.py.sigstore.json'

cosign verify-blob "$temp/$name" --bundle "$temp/$name.sigstore.json" \
  --certificate-identity-regexp "^https://github.com/$repo/.github/workflows/release.yml@refs/tags/v.+$" \
  --certificate-oidc-issuer 'https://github.com/login/oauth' >/dev/null
cosign verify-blob "$temp/$name.manifest.json" --bundle "$temp/$name.manifest.json.sigstore.json" \
  --certificate-identity-regexp "^https://github.com/$repo/.github/workflows/release.yml@refs/tags/$version$" \
  --certificate-oidc-issuer 'https://github.com/login/oauth' >/dev/null
cosign verify-blob "$temp/gent-activate-install.py" --bundle "$temp/gent-activate-install.py.sigstore.json" \
  --certificate-identity-regexp "^https://github.com/$repo/.github/workflows/release.yml@refs/tags/$version$" \
  --certificate-oidc-issuer 'https://github.com/login/oauth' >/dev/null

python3 - "$temp/$name" "$temp/$name.manifest.json" "$temp/$name.sha256" "$version" "$target" <<'PY'
import hashlib, json, pathlib, sys
import tarfile
archive, manifest_path, checksum_path = map(pathlib.Path, sys.argv[1:4])
version, target = sys.argv[4:]
manifest = json.loads(manifest_path.read_text(encoding='utf-8'))
digest = hashlib.sha256(archive.read_bytes()).hexdigest()
expected = manifest.get('archive', {}).get('sha256')
line = f'{expected}  {archive.name}'
if (manifest.get('schemaVersion') != 1 or manifest.get('version') != version
        or manifest.get('target') != target or expected != digest
        or manifest.get('archive', {}).get('size') != archive.stat().st_size
        or checksum_path.read_text(encoding='utf-8').strip() != line):
    raise SystemExit('release archive verification failed')
if sorted(manifest.get('binaries', [])) != ['gent', 'gentd']:
    raise SystemExit('release manifest does not contain gent and gentd')
root = f'gent-{version}-{target}'
with tarfile.open(archive, 'r:gz') as bundle:
    names = sorted(member.name for member in bundle.getmembers())
    if names != [f'{root}/gent', f'{root}/gentd'] or any(not member.isfile() for member in bundle.getmembers()):
        raise SystemExit('release archive contains unsafe or unexpected paths')
PY

if [ -n "$expected_sha256" ]; then
  actual_sha256=$(python3 - "$temp/$name" <<'PY'
import hashlib, pathlib, sys
print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())
PY
)
  [ "$actual_sha256" = "$expected_sha256" ] || { printf '%s\n' 'release digest does not match explicit update confirmation' >&2; exit 1; }
fi

tar -xzf "$temp/$name" -C "$temp"
release_dir="$temp/gent-$version-$target"
[ -x "$release_dir/gent" ] && [ -x "$release_dir/gentd" ] || { printf '%s\n' 'release archive has invalid binaries' >&2; exit 1; }

bin_dir="$install_root/bin"
runtime_root="$install_root/lib/gent"
release_name="$version-$target"
release_path="$runtime_root/releases/$release_name"
if [ "$force" -ne 1 ] && [ -e "$runtime_root/current" ]; then
  printf 'Gent is already installed in %s; pass --force to replace it.\n' "$bin_dir" >&2
  exit 1
fi
mkdir -p "$bin_dir" "$runtime_root/releases"
if [ ! -e "$release_path" ]; then
  stage="$runtime_root/releases/.gent-stage-$$"
  mkdir -p "$stage"
  cp "$release_dir/gent" "$stage/gent"
  cp "$release_dir/gentd" "$stage/gentd"
  chmod 755 "$stage/gent" "$stage/gentd"
  mv "$stage" "$release_path"
fi
if [ -n "$idle_data_dir" ]; then
  python3 "$temp/gent-activate-install.py" "$runtime_root" "$release_name" --idle-data-dir "$idle_data_dir"
else
  python3 "$temp/gent-activate-install.py" "$runtime_root" "$release_name"
fi
for launcher in gent gentd; do
  cat >"$bin_dir/$launcher" <<'SH'
#!/usr/bin/env sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../lib/gent" && pwd)
exec "$root/current/$(basename -- "$0")" "$@"
SH
  chmod 755 "$bin_dir/$launcher"
done
printf 'Installed Gent %s in %s\n' "$version" "$bin_dir"
printf 'Add %s to PATH, then run: gent doctor\n' "$bin_dir"
