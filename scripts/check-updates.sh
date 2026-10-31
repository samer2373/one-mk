#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
python3 - "$root" "$scratch" <<'PY'
import pathlib
import re
import subprocess
import sys
import yaml

root, scratch = map(pathlib.Path, sys.argv[1:])
workflow = yaml.safe_load((root / ".github/workflows/release.yml").read_text())
count = 0
for job in workflow["jobs"].values():
    for step in job.get("steps", []):
        if "run" in step:
            run = re.sub(r"\$\{\{.*?\}\}", "EXPRESSION", step["run"])
            subprocess.run(["bash", "-n"], input=run, text=True, check=True)
            count += 1
steps = workflow["jobs"]["publish"]["steps"]
for name in ["Stage the assets", "Generate and sign update manifest", "Checksum all staged assets including the signed manifest", "Publish"]:
    step = next(step for step in steps if step.get("name") == name)
    (scratch / (name.split()[0] + ".sh")).write_text(step["run"])
print(f"YAML parsed; bash syntax checked for {count} workflow run blocks")
PY
mkdir -p "$scratch/scripts" "$scratch/crates/one-mk/src" "$scratch/dist/homebrew" "$scratch/bin"
cp "$root"/scripts/{version,make-update-manifest,sign-update-manifest,update-keygen}.sh "$scratch/scripts/"
cp "$root/Cargo.toml" "$scratch/Cargo.toml"
cp "$root/crates/one-mk/src/update_key.rs" "$scratch/crates/one-mk/src/update_key.rs"
bash "$root/scripts/update-keygen.sh" "$scratch/test-key" > "$scratch/keygen-output"
openssl pkey -in "$scratch/test-key.pem" -pubout -outform DER -out "$scratch/public.der"
python3 - "$scratch" <<'PY'
import pathlib
import re
import sys

scratch = pathlib.Path(sys.argv[1])
key = (scratch / "public.der").read_bytes()[12:]
path = scratch / "crates/one-mk/src/update_key.rs"
text = path.read_text().replace("PRODUCTION_KEY: bool = false", "PRODUCTION_KEY: bool = true")
text = re.sub(r"PUBLIC_KEY: \[u8; 32\] = \[[^]]+\]", "PUBLIC_KEY: [u8; 32] = [" + ", ".join("0x%02x" % byte for byte in key) + "]", text)
path.write_text(text)
for name in ["one-mk-x86_64-unknown-linux-gnu.bin", "one-mk-x86_64-pc-windows-msvc.exe", "one-mk-aarch64-apple-darwin.bin", "one-mk-x86_64-unknown-linux-gnu.AppImage"]:
    (scratch / "dist" / name).write_bytes(b"local stand-in executable\n")
for name in ["one-mk.rb", "one-mk-source.rb"]:
    (scratch / "dist/homebrew" / name).write_text("local stand-in package metadata\n")
PY
cat > "$scratch/bin/gh" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> gh-calls
case "$1 $2" in
  'release view')
    [ -e draft ] || [ -e published ] || exit 1
    case "$*" in
      *'--json assets'*) find uploaded-assets -maxdepth 1 -type f -printf '%f\n' ;;
      *'--json isDraft'*) if [ -e published ]; then echo false; else echo true; fi ;;
    esac ;;
  'release create') touch draft ;;
  'release upload')
    [ -e draft ] && [ ! -e published ]
    mkdir -p uploaded-assets
    for argument in "$@"; do
      if [ -f "$argument" ]; then cp "$argument" uploaded-assets/; fi
    done ;;
  'release edit') test -e uploaded-assets/SHA256SUMS; touch published ;;
  *) echo 'Unexpected stub gh call' >&2; exit 1 ;;
esac
SH
chmod +x "$scratch/bin/gh"
(
  cd "$scratch"
  export PATH="$scratch/bin:$PATH" GITHUB_REF_NAME=v0.1.12 GITHUB_REPOSITORY=local/test GITHUB_RUN_NUMBER=123 RELEASE_TIME=1793318400 GITHUB_STEP_SUMMARY="$scratch/summary"
  bash Stage.sh
  # With the production key on and no signing key, publishing is refused.
  if bash Generate.sh > unsigned-production-output 2>&1; then echo 'Unsigned production release was accepted' >&2; exit 1; fi
  grep -q 'refusing to publish' unsigned-production-output
  # With the test key still in place, an unsigned release is produced and flagged.
  cp crates/one-mk/src/update_key.rs production-key.rs
  sed -i 's/PRODUCTION_KEY: bool = true/PRODUCTION_KEY: bool = false/' crates/one-mk/src/update_key.rs
  bash Generate.sh
  test ! -e release-files/update-manifest.json.sig
  grep -q UNSIGNED summary
  cp production-key.rs crates/one-mk/src/update_key.rs
  export ONE_MK_UPDATE_SIGNING_KEY
  ONE_MK_UPDATE_SIGNING_KEY=$(cat test-key.pem)
  bash Generate.sh
  unset ONE_MK_UPDATE_SIGNING_KEY
  test "$(wc -c < release-files/update-manifest.json.sig)" -eq 64
  bash Checksum.sh > checksums-output
  (cd release-files && sha256sum -c SHA256SUMS)
  touch notes.md
  bash Publish.sh
  test -e published
  if bash Publish.sh > rerun-output 2>&1; then echo 'Published release was overwritten' >&2; exit 1; fi
  grep -q 'Refusing to overwrite' rerun-output
  cp release-files/update-manifest.json tampered
  printf ' ' >> tampered
  if openssl pkeyutl -verify -rawin -pubin -inkey test-key.pub.pem -in tampered -sigfile release-files/update-manifest.json.sig > tamper-output 2>&1; then
    echo 'Tampered manifest was accepted' >&2; exit 1
  fi
  python3 - <<'PY'
import json
import pathlib

manifest = json.loads(pathlib.Path("release-files/update-manifest.json").read_bytes())
assert manifest["sequence"] == 123
assert len(manifest["assets"]) == 6
assert {asset["os"] for asset in manifest["assets"]} == {"linux", "windows", "macos", "any"}
assert {asset["kind"] for asset in manifest["assets"]} == {"binary", "appimage", "metadata"}
PY
  echo 'Local publish passed: unsigned warning, signed verification, checksums, draft/upload/publish, immutable rerun and tamper refusal (stub gh only).'
)
