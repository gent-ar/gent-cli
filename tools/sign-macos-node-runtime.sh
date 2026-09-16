#!/usr/bin/env bash
set -euo pipefail
node="${1:?Expected the staged Node runtime directory}/bin/node"
entitlements="$(cd "$(dirname "$0")/.." && pwd)/platform/macos/node-runtime.entitlements"
: "${MACOS_CERTIFICATE:?}" "${MACOS_CERTIFICATE_PASSWORD:?}"
: "${MACOS_NOTARY_APPLE_ID:?}" "${MACOS_NOTARY_PASSWORD:?}" "${MACOS_NOTARY_TEAM_ID:?}"
test -f "$node"
umask 077
work="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/gent-node-signing.XXXXXX")"
keychain="$work/signing.keychain-db"
keychain_password="$(uuidgen)"
cleanup() {
  security delete-keychain "$keychain" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT
printf '%s' "$MACOS_CERTIFICATE" | base64 --decode > "$work/certificate.p12"
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 3600 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security default-keychain -s "$keychain"
security import "$work/certificate.p12" -k "$keychain" -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null
rm "$work/certificate.p12"
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
identity="$(security find-identity -v -p codesigning "$keychain" | awk '/"Developer ID Application: /{print $2; exit}')"
test -n "$identity" || { echo "the signing certificate has no Developer ID Application identity" >&2; exit 1; }

codesign --force --sign "$identity" --keychain "$keychain" --options runtime --timestamp --entitlements "$entitlements" "$node"
codesign --verify --strict --verbose=2 "$node"
details="$(codesign -dvv "$node" 2>&1)"
grep -q '^Authority=Developer ID Application: ' <<<"$details"
grep -q '^Timestamp=' <<<"$details"
grep -Eq 'flags=0x[0-9a-f]+\((.*,)?runtime(,.*)?\)' <<<"$details"
granted="$(codesign -d --entitlements - --xml "$node" 2>/dev/null)"
for entitlement in com.apple.security.cs.allow-jit com.apple.security.cs.allow-unsigned-executable-memory; do
  grep -q "$entitlement" <<<"$granted" || { echo "signed Node lacks $entitlement" >&2; exit 1; }
done
if grep -q get-task-allow <<<"$granted"; then
  echo "signed Node must not carry get-task-allow" >&2
  exit 1
fi

ditto -c -k --keepParent "$node" "$work/node.zip"
xcrun notarytool submit "$work/node.zip" --wait --output-format json \
  --apple-id "$MACOS_NOTARY_APPLE_ID" --password "$MACOS_NOTARY_PASSWORD" --team-id "$MACOS_NOTARY_TEAM_ID" > "$work/notary.json" || true
python3 - "$work/notary.json" <<'PY'
import json, sys
result = json.load(open(sys.argv[1], encoding="utf-8"))
print(f"notarization {result.get('id')}: {result.get('status')}")
sys.exit(result.get("status") != "Accepted")
PY
