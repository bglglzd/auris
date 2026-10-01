#!/usr/bin/env bash
# Создаёт бесплатный самоподписанный сертификат подписи кода для Memiro AI (macOS).
#
# Зачем: с постоянной подписью macOS узнаёт приложение после обновления и НЕ
# спрашивает заново доступ к микрофону и звуку собеседников. (Временная ad-hoc
# подпись меняется с каждой сборкой — разрешения сбрасывались.) Предупреждение
# «не удаётся проверить разработчика» при первом открытии остаётся — его
# убирает только платный Apple Developer ID.
#
# Запуск (macOS, Linux или Git Bash на Windows; нужен openssl):
#   scripts/make-macos-signing-cert.sh
# Затем добавьте два секрета в GitHub → репозиторий → Settings → Secrets and
# variables → Actions → New repository secret:
#   MACOS_CERT_P12       — содержимое файла memiro-signing.p12.base64
#   MACOS_CERT_PASSWORD  — пароль, который выведет скрипт
# Файлы сертификата храните в надёжном месте и не добавляйте в git: тот, у кого
# есть ключ, может подписать приложение «от имени» Memiro.
#
# Для CI-проверки: OUT_DIR=<папка> PASSWORD=<пароль> QUIET=1 — без подсказок.
set -euo pipefail

NAME="Memiro AI Self-Signed"
OUT="${OUT_DIR:-.}"
PASS="${PASSWORD:-$(openssl rand -hex 16)}"
mkdir -p "$OUT"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

cat > "$TMP/cert.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
O = Memiro AI
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
CNF

openssl req -x509 -newkey rsa:2048 -sha256 -days 3650 -nodes \
  -keyout "$TMP/key.pem" -out "$TMP/cert.pem" -config "$TMP/cert.cnf" 2>/dev/null

# PKCS#12 в «старом» формате (3DES/SHA1) — его понимает `security import` macOS.
openssl pkcs12 -export -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -name "$NAME" \
  -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1 \
  -out "$OUT/memiro-signing.p12" -passout "pass:$PASS"
base64 < "$OUT/memiro-signing.p12" | tr -d '\n' > "$OUT/memiro-signing.p12.base64"

if [ -z "${QUIET:-}" ]; then
  echo "Готово: $OUT/memiro-signing.p12 (+ .base64)"
  echo
  echo "Добавьте в GitHub → Settings → Secrets and variables → Actions:"
  echo "  MACOS_CERT_P12      = содержимое $OUT/memiro-signing.p12.base64"
  echo "  MACOS_CERT_PASSWORD = $PASS"
  echo
  echo "Отпечаток сертификата:"
  openssl x509 -in "$TMP/cert.pem" -noout -fingerprint -sha1
else
  echo "$PASS" > "$OUT/memiro-signing.password"
fi
