#!/bin/sh
# Developer ID signing and notarization; local development stays ad-hoc.
set -eu
APP=${1:?Usage: sign-macos.sh path/to/InputMethod.app}
IDENTITY=${MACOS_SIGN_IDENTITY:-}
if [ -z "$IDENTITY" ]; then
    if [ -n "${MACOS_NOTARY_KEY:-}${MACOS_NOTARY_PROFILE:-}" ]; then
        echo "Notarization requires MACOS_SIGN_IDENTITY" >&2
        exit 1
    fi
    codesign --force --sign - "$APP"
else
    codesign --force --options runtime --timestamp --sign "$IDENTITY" "$APP"
fi
codesign --verify --strict "$APP"

if [ -n "${MACOS_NOTARY_KEY:-}${MACOS_NOTARY_PROFILE:-}" ]; then
    WORK=$(mktemp -d)
    trap 'rm -rf "$WORK"' EXIT
    ditto -c -k --sequesterRsrc --keepParent "$APP" "$WORK/app.zip"
    if [ -n "${MACOS_NOTARY_PROFILE:-}" ]; then
        xcrun notarytool submit "$WORK/app.zip" --keychain-profile "$MACOS_NOTARY_PROFILE" --wait --timeout 20m --output-format json > "$WORK/result.json"
    else
        : "${MACOS_NOTARY_KEY_ID:?Missing notary key ID}"
        : "${MACOS_NOTARY_ISSUER_ID:?Missing notary issuer ID}"
        xcrun notarytool submit "$WORK/app.zip" --key "$MACOS_NOTARY_KEY" --key-id "$MACOS_NOTARY_KEY_ID" --issuer "$MACOS_NOTARY_ISSUER_ID" --wait --timeout 20m --output-format json > "$WORK/result.json"
    fi
    python3 - "$WORK/result.json" <<'PY'
import json, sys
with open(sys.argv[1]) as source:
    result = json.load(source)
if result.get('status') != 'Accepted':
    raise SystemExit('Apple notarization failed: ' + str(result.get('id')) + ' / ' + str(result.get('status')))
PY
    xcrun stapler staple "$APP"
    xcrun stapler validate "$APP"
    codesign --verify --strict "$APP"
    spctl --assess --type execute "$APP"
fi
