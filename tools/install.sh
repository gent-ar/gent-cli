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
python3 - "$version" <<'PY'
import re
import sys

if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?", sys.argv[1]):
    raise SystemExit(f"invalid release version: {sys.argv[1]}")
PY
case "$expected_sha256" in *[!0-9a-f]*) printf '%s\n' 'expected digest must be lowercase hexadecimal' >&2; exit 1 ;; esac
[ -z "$expected_sha256" ] || [ "${#expected_sha256}" -eq 64 ] || { printf '%s\n' 'expected digest must contain 64 hexadecimal characters' >&2; exit 1; }
tag_identity_regex=$(python3 -c 'import re,sys; print(re.escape(sys.argv[1]))' "$version")

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
auto_update_base="$release_base/gent-auto-update.py"
download "$auto_update_base" 'gent-auto-update.py'
download "$auto_update_base.sigstore.json" 'gent-auto-update.py.sigstore.json'
supervisor_base="$release_base/gent-supervise-runtime-activation.py"
has_supervisor=0
if download "$supervisor_base" 'gent-supervise-runtime-activation.py' 2>/dev/null; then
  download "$supervisor_base.sigstore.json" 'gent-supervise-runtime-activation.py.sigstore.json' || exit 1
  has_supervisor=1
fi
if [ -n "$idle_data_dir" ] && [ "$has_supervisor" -ne 1 ]; then
  printf '%s\n' 'signed runtime activation supervisor is required for an update handoff' >&2
  exit 1
fi
trust_base="$release_base/gent-runtime-release-trust.json"
release_metadata="gent-$version-$target.runtime-release.json"
has_update_material=0
if download "$trust_base" 'gent-runtime-release-trust.json' 2>/dev/null; then
  download "$trust_base.sigstore.json" 'gent-runtime-release-trust.json.sigstore.json'
  download "$release_base/$release_metadata" "$release_metadata"
  download "$release_base/$release_metadata.sigstore.json" "$release_metadata.sigstore.json"
  has_update_material=1
fi

cosign verify-blob "$temp/$name" --bundle "$temp/$name.sigstore.json" \
  --certificate-identity-regexp "^https://github.com/$repo/.github/workflows/release.yml@refs/tags/$tag_identity_regex$" \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' >/dev/null
cosign verify-blob "$temp/$name.manifest.json" --bundle "$temp/$name.manifest.json.sigstore.json" \
  --certificate-identity-regexp "^https://github.com/$repo/.github/workflows/release.yml@refs/tags/$tag_identity_regex$" \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' >/dev/null
cosign verify-blob "$temp/gent-activate-install.py" --bundle "$temp/gent-activate-install.py.sigstore.json" \
  --certificate-identity-regexp "^https://github.com/$repo/.github/workflows/release.yml@refs/tags/$tag_identity_regex$" \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' >/dev/null
cosign verify-blob "$temp/gent-auto-update.py" --bundle "$temp/gent-auto-update.py.sigstore.json" \
  --certificate-identity-regexp "^https://github.com/$repo/.github/workflows/release.yml@refs/tags/$tag_identity_regex$" \
  --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' >/dev/null
if [ "$has_supervisor" -eq 1 ]; then
  cosign verify-blob "$temp/gent-supervise-runtime-activation.py" --bundle "$temp/gent-supervise-runtime-activation.py.sigstore.json" \
    --certificate-identity-regexp "^https://github.com/$repo/.github/workflows/release.yml@refs/tags/$tag_identity_regex$" \
    --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' >/dev/null
fi
if [ "$has_update_material" -eq 1 ]; then
  cosign verify-blob "$temp/gent-runtime-release-trust.json" --bundle "$temp/gent-runtime-release-trust.json.sigstore.json" \
    --certificate-identity-regexp "^https://github.com/$repo/.github/workflows/release.yml@refs/tags/$tag_identity_regex$" \
    --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' >/dev/null
  cosign verify-blob "$temp/$release_metadata" --bundle "$temp/$release_metadata.sigstore.json" \
    --certificate-identity-regexp "^https://github.com/$repo/.github/workflows/release.yml@refs/tags/$tag_identity_regex$" \
    --certificate-oidc-issuer 'https://token.actions.githubusercontent.com' >/dev/null
  mkdir "$temp/update-material"
  cp "$temp/gent-runtime-release-trust.json" "$temp/update-material/runtime-release-trust.json"
fi

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
required_capabilities = {
    'agent-chat-conversations-v1', 'agent-chat-intents-v1', 'agent-chat-transcript-v1',
    'agent-chat-turn-follow-v1', 'agent-chat-permissions-v1', 'attachments-v1', 'local-models-v1',
}
capabilities = manifest.get('capabilities', [])
if (sorted(manifest.get('binaries', [])) != ['gent', 'gentd']
        or manifest.get('runtimes') != ['runtime/node', 'runtime/claurst']
        or not isinstance(capabilities, list) or not required_capabilities.issubset(capabilities)):
    raise SystemExit('release manifest does not contain gent and gentd')
root = f'gent-{version}-{target}'
with tarfile.open(archive, 'r:gz') as bundle:
    names = sorted(member.name for member in bundle.getmembers())
    required = [
        f'{root}/gent', f'{root}/gentd', f'{root}/runtime/node/bin/node',
        f'{root}/runtime/node/bin/npm',
        f'{root}/runtime/node/lib/node_modules/npm/bin/npm-cli.js',
        f'{root}/runtime/claurst/claurst', f'{root}/runtime/claurst/llama/llama-server',
    ]
    if (any(not member.isfile() for member in bundle.getmembers()) or
            any(name not in required and not name.startswith(f'{root}/runtime/node/') and not name.startswith(f'{root}/runtime/claurst/llama/') for name in names) or
            any(name not in names for name in required)):
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
[ -x "$release_dir/gent" ] && [ -x "$release_dir/gentd" ] && [ -f "$release_dir/runtime/node/bin/node" ] && [ -f "$release_dir/runtime/node/bin/npm" ] && [ -f "$release_dir/runtime/node/lib/node_modules/npm/bin/npm-cli.js" ] && [ -x "$release_dir/runtime/claurst/claurst" ] && [ -x "$release_dir/runtime/claurst/llama/llama-server" ] || { printf '%s\n' 'release archive has invalid runtime files' >&2; exit 1; }
if [ "$has_update_material" -eq 1 ]; then
  "$release_dir/gentd" --verify-runtime-update-material \
    --runtime-release-cache "$temp/update-material/runtime-release-cache.json" \
    --runtime-release-trust "$temp/update-material/runtime-release-trust.json" \
    --runtime-release-manifest "$temp/$release_metadata" \
    --runtime-release-archive "$temp/$name" \
    --runtime-release-archive-manifest "$temp/$name.manifest.json"
fi

bin_dir="$install_root/bin"
runtime_root="$install_root/lib/gent"
release_name="$version-$target"
install_auto_updater() {
  temporary="$runtime_root/.gent-auto-update.$$"
  cp "$temp/gent-auto-update.py" "$temporary"
  chmod 700 "$temporary"
  mv -f "$temporary" "$runtime_root/gent-auto-update.py"
}
enable_auto_updates() {
  [ -e "$runtime_root/.gent-auto-update-disabled" ] && return
  "$bin_dir/gent" update auto enable
}
if [ -n "$idle_data_dir" ] && [ -L "$runtime_root/current" ]; then
  set -- python3 "$temp/gent-supervise-runtime-activation.py" \
    --runtime-root "$runtime_root" --release-name "$release_name" \
    --source-release "$release_dir" --bin-dir "$bin_dir" --data-dir "$idle_data_dir" \
    --activator "$temp/gent-activate-install.py" --source-auto-updater "$temp/gent-auto-update.py" \
    --timeout-seconds "${GENT_RUNTIME_ACTIVATION_TIMEOUT_SECONDS:-30}"
  if [ "$has_update_material" -eq 1 ]; then
    set -- "$@" --source-update-material "$temp/update-material"
  fi
  "$@"
  install_auto_updater
  enable_auto_updates
  printf 'Updated Gent %s in %s\n' "$version" "$bin_dir"
  exit 0
fi
set -- "$runtime_root" "$release_name" --source-release "$release_dir" --bin-dir "$bin_dir"
if [ "$has_supervisor" -eq 1 ]; then
  set -- "$@" --source-supervisor "$temp/gent-supervise-runtime-activation.py"
fi
set -- "$@" --source-auto-updater "$temp/gent-auto-update.py"
if [ "$has_update_material" -eq 1 ]; then
  set -- "$@" --source-update-material "$temp/update-material"
fi
if [ "$force" -eq 1 ]; then
  set -- "$@" --force
fi
if [ -n "$idle_data_dir" ]; then
  set -- "$@" --idle-data-dir "$idle_data_dir"
fi
python3 "$temp/gent-activate-install.py" "$@"
install_auto_updater
enable_auto_updates
printf 'Installed Gent %s in %s\n' "$version" "$bin_dir"
printf 'Add %s to PATH, then run: gent doctor\n' "$bin_dir"
