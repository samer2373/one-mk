#!/usr/bin/env bash
set -euo pipefail
umask 077
if [ "$#" -ne 1 ]; then
  echo 'Usage: bash scripts/update-keygen.sh /secure/path/one-mk-update (new path only)' >&2
  exit 2
fi
if [ -e "$1.pem" ] || [ -e "$1.pub.pem" ]; then
  echo 'Key files already exist; choose a new path. Existing keys were not changed.' >&2
  exit 1
fi
set -o noclobber
openssl genpkey -algorithm ed25519 > "$1.pem"
openssl pkey -in "$1.pem" -pubout > "$1.pub.pem"
openssl pkey -in "$1.pem" -pubout -outform DER | python3 -c '
import sys
public = sys.stdin.buffer.read()
if len(public) != 44 or public[:12].hex() != "302a300506032b6570032100":
    sys.exit("Unexpected Ed25519 public key format")
key = public[12:]
print("Public key hex:", key.hex())
print("pub const PUBLIC_KEY: [u8; 32] = [" + ", ".join("0x%02x" % byte for byte in key) + "];")
'
echo "Private key saved to $1.pem (0600). Keep it offline; never commit it."
echo 'Replace the public key and set PRODUCTION_KEY = true before releasing.'
