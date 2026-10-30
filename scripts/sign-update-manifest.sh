#!/usr/bin/env bash
set -euo pipefail
if [ "$#" -ne 1 ]; then
  echo 'Usage: bash scripts/sign-update-manifest.sh ASSET_DIRECTORY' >&2
  exit 2
fi
directory=$1
if [ -z "${ONE_MK_UPDATE_SIGNING_KEY:-}" ]; then
  if grep -Eq 'PRODUCTION_KEY:[[:space:]]*bool[[:space:]]*=[[:space:]]*true' crates/one-mk/src/update_key.rs; then
    echo 'PRODUCTION_KEY is on but ONE_MK_UPDATE_SIGNING_KEY is not configured: refusing to publish a release that trusts the production key but is unsigned.' >&2
    exit 1
  fi
  rm -f "$directory/update-manifest.json.sig"
  echo 'UNSIGNED release: ONE_MK_UPDATE_SIGNING_KEY is not configured. The updater refuses it.' | tee -a "${GITHUB_STEP_SUMMARY:-/dev/stdout}"
  exit 0
fi
umask 077
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
printf '%s\n' "$ONE_MK_UPDATE_SIGNING_KEY" > "$scratch/private.pem"
unset ONE_MK_UPDATE_SIGNING_KEY
python3 - "$scratch/public.der" <<'PY'
import pathlib
import re
import sys

source = pathlib.Path("crates/one-mk/src/update_key.rs").read_text()
if not re.search(r"PRODUCTION_KEY:\s*bool\s*=\s*true\s*;", source):
    sys.exit("Refusing to sign a release built with a placeholder update key")
match = re.search(r"PUBLIC_KEY:\s*\[u8;\s*32\]\s*=\s*\[([^]]+)\]", source)
if match is None:
    sys.exit("Cannot locate embedded update public key")
values = re.findall(r"0x([0-9a-fA-F]{2})", match.group(1))
if len(values) != 32:
    sys.exit("Embedded update public key must contain exactly 32 bytes")
pathlib.Path(sys.argv[1]).write_bytes(bytes.fromhex("302a300506032b6570032100" + "".join(values)))
PY
openssl pkey -pubin -inform DER -in "$scratch/public.der" -out "$scratch/public.pem"
openssl pkeyutl -sign -rawin -inkey "$scratch/private.pem" -in "$directory/update-manifest.json" -out "$scratch/signature"
openssl pkeyutl -verify -rawin -pubin -inkey "$scratch/public.pem" -in "$directory/update-manifest.json" -sigfile "$scratch/signature"
test "$(wc -c < "$scratch/signature")" -eq 64
cp "$scratch/signature" "$directory/update-manifest.json.sig"
echo 'Ed25519 update manifest signed and verified against the compiled public key.' >> "${GITHUB_STEP_SUMMARY:-/dev/stdout}"
