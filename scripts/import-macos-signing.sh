#!/bin/sh
# CI-only ephemeral keychain. Secrets enter through environment variables.
set -eu
: "${RUNNER_TEMP:?CI runner required}"
: "${GITHUB_ENV:?CI environment file required}"
if [ -z "${MACOS_CERTIFICATE_P12:-}" ]; then
    if [ -n "${MACOS_SIGN_IDENTITY:-}${MACOS_NOTARY_KEY_BASE64:-}" ]; then
        echo "Apple signing requires MACOS_CERTIFICATE_P12" >&2
        exit 1
    fi
    echo "No Apple signing identity configured; building ad-hoc artifacts."
    exit 0
fi
: "${MACOS_CERTIFICATE_PASSWORD:?Missing P12 password}"
: "${MACOS_SIGN_IDENTITY:?Missing signing identity}"
KEYCHAIN="$RUNNER_TEMP/zoo-type-signing.keychain-db"
CERTIFICATE="$RUNNER_TEMP/zoo-type-signing.p12"
umask 077
KEYCHAIN_PASSWORD=$(uuidgen)
export CERTIFICATE
python3 - <<'PY'
import base64, os
with open(os.environ['CERTIFICATE'], 'wb') as output:
    output.write(base64.b64decode(os.environ['MACOS_CERTIFICATE_P12'], validate=True))
PY
security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security set-keychain-settings -lut 21600 "$KEYCHAIN"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security import "$CERTIFICATE" -k "$KEYCHAIN" -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
security list-keychains -d user -s "$KEYCHAIN" login.keychain-db
rm -f "$CERTIFICATE"
printf 'MACOS_KEYCHAIN_PATH=%s\nMACOS_SIGN_IDENTITY=%s\n' "$KEYCHAIN" "$MACOS_SIGN_IDENTITY" >> "$GITHUB_ENV"
if [ -n "${MACOS_NOTARY_KEY_BASE64:-}" ]; then
    : "${MACOS_NOTARY_KEY_ID:?Missing notary key ID}"
    : "${MACOS_NOTARY_ISSUER_ID:?Missing notary issuer ID}"
    export MACOS_NOTARY_KEY="$RUNNER_TEMP/zoo-type-notary.p8"
    python3 - <<'PY'
import base64, os
with open(os.environ['MACOS_NOTARY_KEY'], 'wb') as output:
    output.write(base64.b64decode(os.environ['MACOS_NOTARY_KEY_BASE64'], validate=True))
PY
    chmod 600 "$MACOS_NOTARY_KEY"
    printf 'MACOS_NOTARY_KEY=%s\nMACOS_NOTARY_KEY_ID=%s\nMACOS_NOTARY_ISSUER_ID=%s\n' "$MACOS_NOTARY_KEY" "$MACOS_NOTARY_KEY_ID" "$MACOS_NOTARY_ISSUER_ID" >> "$GITHUB_ENV"
fi
