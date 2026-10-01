#!/usr/bin/env bash
# CI (macOS): импортирует сертификат подписи во временный keychain и отдаёт его
# отпечаток Tauri через APPLE_SIGNING_IDENTITY (в $GITHUB_ENV).
#   P12_BASE64=<base64 .p12> P12_PASSWORD=<пароль> scripts/import-macos-signing-cert.sh
# Самоподписанный сертификат Tauri сам не импортирует (ищет только сертификаты
# Apple), поэтому импорт здесь, а codesign получает отпечаток SHA-1.
set -euo pipefail
: "${P12_BASE64:?}" "${P12_PASSWORD:?}"

WORK="${RUNNER_TEMP:-$(mktemp -d)}"
KC="$WORK/memiro-signing.keychain-db"
KC_PASS="$(openssl rand -hex 16)"
P12="$WORK/memiro-signing.p12"
printf '%s' "$P12_BASE64" | base64 --decode > "$P12"

security create-keychain -p "$KC_PASS" "$KC"
security set-keychain-settings -lut 21600 "$KC"
security unlock-keychain -p "$KC_PASS" "$KC"
security import "$P12" -k "$KC" -P "$P12_PASSWORD" -f pkcs12 -T /usr/bin/codesign -T /usr/bin/security
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KC_PASS" "$KC" > /dev/null
# Временный keychain — в список поиска (вместе с прежними).
security list-keychains -d user -s "$KC" $(security list-keychains -d user | tr -d '"')

HASH="$(openssl pkcs12 -in "$P12" -nokeys -passin "pass:$P12_PASSWORD" 2>/dev/null \
  | openssl x509 -noout -fingerprint -sha1 | sed 's/.*=//; s/://g')"
rm -f "$P12"
test -n "$HASH"
echo "Подпись: самоподписанный сертификат $HASH"
echo "APPLE_SIGNING_IDENTITY=$HASH" >> "${GITHUB_ENV:-/dev/stdout}"
