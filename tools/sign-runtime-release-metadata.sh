#!/usr/bin/env bash
set -eo pipefail
tools=$(dirname "$0")
test -n "$RUNTIME_RELEASE_PRIVATE_KEY"
test -n "$RUNTIME_RELEASE_KEY_ID"
case "$RUNTIME_RELEASE_PUBLIC_KEY" in (????????????????????????????????????????????????????????????????) ;; (*) exit 1 ;; esac
case "$RUNTIME_RELEASE_PUBLIC_KEY" in (*[!0-9a-f]*) exit 1 ;; esac
key="$RUNNER_TEMP/gent-runtime-release.pem"
umask 077
printf '%s\n' "$RUNTIME_RELEASE_PRIVATE_KEY" > "$key"
derived_key=$(python - "$key" "$tools/sign-runtime-release.py" <<'PY'
import importlib.util, pathlib, sys
path = pathlib.Path(sys.argv[2])
spec = importlib.util.spec_from_file_location('runtime_release_signer', path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
print(module.public_key(module.load_seed(pathlib.Path(sys.argv[1]))).hex())
PY
)
test "$derived_key" = "$RUNTIME_RELEASE_PUBLIC_KEY"
expires_at=$(date -u --date '+90 days' +%s)
while IFS= read -r manifest; do
  target=$(python -c 'import json,sys; print(json.load(open(sys.argv[1]))["target"])' "$manifest")
  output="$(dirname "$manifest")/gent-$RELEASE_TAG-$target.runtime-release.json"
  python "$tools/sign-runtime-release.py" \
    --archive-manifest "$manifest" --version "$RELEASE_TAG" --target "$target" \
    --key-id "$RUNTIME_RELEASE_KEY_ID" --private-key "$key" --expires-at "$expires_at" \
    --protocol-min 1 --protocol-max 1 --schema-max 23 \
    --minimum-app-version "${RELEASE_TAG#v}" --out "$output"
done < <(find release-artifacts -name '*.manifest.json' -type f | sort)
set -- --key-id "$RUNTIME_RELEASE_KEY_ID" --private-key "$key" --expires-at "$expires_at" \
  --out "release-artifacts/gent-$RELEASE_TAG.runtime-release-index.json"
while IFS= read -r release; do
  set -- "$@" --runtime-release "$release"
done < <(find release-artifacts -name '*.runtime-release.json' -type f | sort)
python "$tools/sign-runtime-index.py" "$@"
python - <<'PY'
import json, os, pathlib
pathlib.Path('release-artifacts/gent-runtime-release-trust.json').write_text(json.dumps({
    'schemaVersion': 1,
    'keys': [{'keyId': os.environ['RUNTIME_RELEASE_KEY_ID'], 'publicKeyHex': os.environ['RUNTIME_RELEASE_PUBLIC_KEY']}],
}, separators=(',', ':')) + '\n', encoding='utf-8')
PY
rm -f "$key"
find release-artifacts \( -name '*.runtime-release.json' -o -name '*.runtime-release-index.json' -o -name 'gent-runtime-release-trust.json' \) -type f -print0 \
  | xargs -0 -n1 sh -c 'cosign sign-blob --yes --bundle "$0.sigstore.json" "$0"'
