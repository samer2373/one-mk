#!/usr/bin/env python3
"""Tests scripts/make-update-manifest.sh with the file names the release really makes.

    python3 scripts/test-update-manifest.py

The updater picks its download from the signed list by (system, architecture, kind):
exactly one "binary" for a portable install, one "appimage", one "deb" for a package,
one ".app.zip" "app" for a Mac. This builds the list from stand-ins for every file
release.yml produces (names taken from release.yml itself) and checks that each of
those choices is exactly one file, that the list is byte-for-byte reproducible, and
that hostile or wrong input is refused. Also signs the list with a throwaway key and
checks that a good signature verifies and every damaged one does not. Needs OpenSSL 3.
"""
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("consistency", REPO / "scripts/check-release-consistency.py")
consistency = importlib.util.module_from_spec(spec)
spec.loader.exec_module(consistency)

ENVIRONMENT = {**os.environ, "RELEASE_TIME": "1793318400"}
for variable in ("ONE_MK_REVOKED", "ONE_MK_MINIMUM_SUPPORTED_VERSION"):
    ENVIRONMENT.pop(variable, None)


def make(directory, sequence="123", **environment):
    return subprocess.run(["bash", "scripts/make-update-manifest.sh", str(directory), str(sequence)], cwd=REPO,
                          env={**ENVIRONMENT, **environment}, capture_output=True, text=True)


def release_names():
    class Quiet(consistency.Report):
        def ok(self, text): pass
        def bad(self, text): raise AssertionError(text)
    report = Quiet()
    produced, _ = consistency.produced_files(REPO, report)
    # Made by the publish job beside them.
    return sorted(produced | {"one-mk.rb", "one-mk-source.rb"})


class Manifest(unittest.TestCase):
    def setUp(self):
        self.scratch = Path(tempfile.mkdtemp(prefix="one-mk-manifest-"))
        self.addCleanup(shutil.rmtree, self.scratch, ignore_errors=True)
        self.stage = self.scratch / "release-files"
        self.stage.mkdir()

    def put(self, name, content=b"stand-in\n"):
        (self.stage / name).write_bytes(content)

    def stage_release(self):
        for name in release_names():
            self.put(name, f"contents of {name}\n".encode())

    def manifest(self):
        return json.loads((self.stage / "update-manifest.json").read_bytes())

    # ------------------------------------------------------------- what the updater selects
    def test_every_choice_the_updater_makes_is_exactly_one_file(self):
        self.stage_release()
        result = make(self.stage)
        self.assertEqual(result.returncode, 0, result.stderr)
        assets = self.manifest()["assets"]
        by_name = {asset["name"]: asset for asset in assets}
        self.assertEqual(set(by_name), set(release_names()), "every staged file is listed, and nothing else")
        selectors = {}
        for asset in assets:
            selectors.setdefault((asset["os"], asset["arch"], asset["kind"]), []).append(asset["name"])
        expected_once = [("linux", "x86_64", "binary"), ("linux", "x86_64", "appimage"), ("linux", "x86_64", "deb"),
                         ("macos", "aarch64", "binary"), ("macos", "x86_64", "binary"), ("windows", "x86_64", "binary")]
        for key in expected_once:
            self.assertEqual(len(selectors.get(key, [])), 1, f"{key}: {selectors.get(key)}")
        # A Mac's app update: one .app.zip of kind "app" per architecture (the .dmg beside it is not chosen).
        for arch in ("aarch64", "x86_64"):
            zips = [a["name"] for a in assets if a["os"] == "macos" and a["arch"] == arch and a["kind"] == "app" and a["name"].endswith(".app.zip")]
            self.assertEqual(zips, [f"one-mk-{arch}-apple-darwin.app.zip"])
        # Classification of every name the release makes.
        expected = {
            "one-mk-x86_64-unknown-linux-gnu.deb": ("linux", "x86_64", "deb"),
            "one-mk-x86_64-unknown-linux-gnu.AppImage": ("linux", "x86_64", "appimage"),
            "one-mk-x86_64-unknown-linux-gnu.tar.gz": ("linux", "x86_64", "archive"),
            "one-mk-x86_64-unknown-linux-gnu.bin": ("linux", "x86_64", "binary"),
            "one-mk-aarch64-apple-darwin.dmg": ("macos", "aarch64", "app"),
            "one-mk-aarch64-apple-darwin.app.zip": ("macos", "aarch64", "app"),
            "one-mk-x86_64-pc-windows-msvc-setup.exe": ("windows", "x86_64", "installer"),
            "one-mk-x86_64-pc-windows-msvc.exe": ("windows", "x86_64", "binary"),
            "one-mk-x86_64-pc-windows-msvc.zip": ("windows", "x86_64", "archive"),
            "one-mk-flatpak-sources.tar.gz": ("any", "any", "archive"),
            "one-mk.rb": ("any", "any", "metadata"),
        }
        for name, (system, arch, kind) in expected.items():
            if name in by_name:
                self.assertEqual((by_name[name]["os"], by_name[name]["arch"], by_name[name]["kind"]), (system, arch, kind), name)
        # The installer is not something the updater may install over a running program.
        self.assertNotIn("installer", {kind for (system, _, kind), names in selectors.items() if system != "windows"})

    def test_the_list_is_the_same_bytes_every_time_and_in_its_documented_shape(self):
        self.stage_release()
        make(self.stage)
        first = (self.stage / "update-manifest.json").read_bytes()
        (self.stage / "update-manifest.json").unlink()
        make(self.stage)
        self.assertEqual((self.stage / "update-manifest.json").read_bytes(), first)
        self.assertTrue(first.endswith(b"}\n") and b"\n" not in first[:-1], "one compact line")
        data = json.loads(first)
        self.assertEqual(list(data), sorted(data), "keys sorted")
        self.assertEqual(data["release_time"], "2026-10-30T00:00:00Z", "RELEASE_TIME is what the list carries")
        self.assertRegex(data["release_time"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
        self.assertEqual(data["sequence"], 123)
        self.assertEqual(data["version"], subprocess.run(["sh", "scripts/version.sh"], cwd=REPO, capture_output=True, text=True).stdout.strip())
        for asset in data["assets"]:
            content = (self.stage / asset["name"]).read_bytes()
            self.assertEqual((asset["size"], asset["sha256"]), (len(content), hashlib.sha256(content).hexdigest()))
        self.assertEqual([a["name"] for a in data["assets"]], sorted(a["name"] for a in data["assets"]), "assets sorted by name")
        self.assertLess(len(first), 256 * 1024)
        # install.sh finds an archive's signed checksum by this exact run of keys (name, os, sha256 side by side).
        for asset in data["assets"]:
            self.assertIn(f'"name":"{asset["name"]}","os":"{asset["os"]}","sha256":"{asset["sha256"]}"'.encode(), first)

    def test_a_staged_manifest_or_checksum_file_is_never_listed_in_itself(self):
        self.stage_release()
        for name in ("SHA256SUMS", "update-manifest.json", "update-manifest.json.sig"):
            self.put(name, b"old\n")
        make(self.stage)
        self.assertFalse({"SHA256SUMS", "update-manifest.json", "update-manifest.json.sig"} & {a["name"] for a in self.manifest()["assets"]})

    # ------------------------------------------------------------- refusals
    def refused(self, why, sequence="123", **environment):
        result = make(self.stage, sequence, **environment)
        self.assertNotEqual(result.returncode, 0, f"{why}: accepted\n{result.stdout}")
        self.assertFalse((self.stage / "update-manifest.json").exists(), f"{why}: a manifest was written anyway")
        return result.stderr

    def test_unsafe_empty_oversized_and_too_many_files_are_refused(self):
        self.put("one-mk-x86_64-unknown-linux-gnu.bin")
        for name in ["has space.bin", "semi;colon.bin", ".hidden", "-dash.bin", "a" * 161, "tab\there.bin", "uml\u00e4ut.bin", "double..dot.bin"]:
            self.put(name)
            self.assertIn("Unsafe staged asset", self.refused(f"the file name {name!r}"))
            (self.stage / name).unlink()
        os.symlink("/etc/passwd", self.stage / "link.bin")
        self.assertIn("Unsafe staged asset", self.refused("a symbolic link"))
        (self.stage / "link.bin").unlink()
        (self.stage / "folder.bin").mkdir()
        self.assertIn("Unsafe staged asset", self.refused("a folder"))
        (self.stage / "folder.bin").rmdir()
        self.put("empty.bin", b"")
        self.assertIn("size limits", self.refused("a zero-byte file"))
        (self.stage / "empty.bin").unlink()
        with open(self.stage / "huge.bin", "wb") as huge:
            huge.truncate(512 * 1024 * 1024 + 1)
        self.assertIn("size limits", self.refused("a file over 512 MiB"))
        (self.stage / "huge.bin").unlink()
        for index in range(128):
            self.put(f"f{index:03}.bin")
        self.assertIn("between 1 and 128", self.refused("129 files"))
        shutil.rmtree(self.stage)
        self.stage.mkdir()
        self.assertIn("between 1 and 128", self.refused("no files"))

    def test_the_sequence_must_be_a_positive_u64(self):
        self.put("one-mk-x86_64-unknown-linux-gnu.bin")
        for sequence in ["0", "-1", "", "abc", "1.5", "1e3", " 7", "7 ", "\u0663", "18446744073709551616", "+5", "0x10"]:
            self.assertIn("sequence", self.refused(f"sequence {sequence!r}", sequence=sequence).lower())
        for sequence in ["1", "18446744073709551615", "007"]:
            self.assertEqual(make(self.stage, sequence).returncode, 0, sequence)
            self.assertEqual(self.manifest()["sequence"], int(sequence))
            (self.stage / "update-manifest.json").unlink()

    def test_revocation_and_minimum_version_inputs_are_checked(self):
        self.put("one-mk-x86_64-unknown-linux-gnu.bin")
        version = subprocess.run(["sh", "scripts/version.sh"], cwd=REPO, capture_output=True, text=True).stdout.strip()
        for what, environment in [
            ("revoking the release itself", {"ONE_MK_REVOKED": version}), ("a word", {"ONE_MK_REVOKED": "latest"}), ("a partial version", {"ONE_MK_REVOKED": "1.2"}),
            ("a leading zero", {"ONE_MK_REVOKED": "01.2.3"}), ("129 versions", {"ONE_MK_REVOKED": " ".join(f"0.{n}.0" for n in range(129))}),
            ("a newer minimum than the release", {"ONE_MK_MINIMUM_SUPPORTED_VERSION": "99.0.0"}), ("a partial minimum", {"ONE_MK_MINIMUM_SUPPORTED_VERSION": "0.1"}),
            ("a v-prefixed minimum", {"ONE_MK_MINIMUM_SUPPORTED_VERSION": "v0.1.0"}),
        ]:
            self.refused(what, **environment)
        result = make(self.stage, ONE_MK_REVOKED="0.1.10, 0.1.9,0.1.9\n0.0.1", ONE_MK_MINIMUM_SUPPORTED_VERSION="0.1.5")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.manifest()["revoked"], ["0.0.1", "0.1.9", "0.1.10"], "sorted by version, not as text, without duplicates")
        self.assertEqual(self.manifest()["minimum_supported_version"], "0.1.5")

    def test_a_release_time_that_is_not_a_time_is_refused(self):
        self.put("one-mk-x86_64-unknown-linux-gnu.bin")
        for value in ["yesterday", "1.5", "99999999999999999999"]:
            self.refused(f"RELEASE_TIME {value!r}", RELEASE_TIME=value)


@unittest.skipUnless(shutil.which("openssl") and subprocess.run(["openssl", "version"], capture_output=True, text=True).stdout.startswith("OpenSSL 3"), "needs OpenSSL 3")
class Signature(unittest.TestCase):
    """The updater and install.sh check this signature with `openssl pkeyutl -verify -rawin` against the compiled-in key."""

    @classmethod
    def setUpClass(cls):
        cls.scratch = Path(tempfile.mkdtemp(prefix="one-mk-signature-"))
        for name in ("one", "two"):
            subprocess.run(["openssl", "genpkey", "-algorithm", "ed25519", "-out", cls.scratch / f"{name}.pem"], check=True, capture_output=True)
            subprocess.run(["openssl", "pkey", "-in", cls.scratch / f"{name}.pem", "-pubout", "-out", cls.scratch / f"{name}.pub"], check=True, capture_output=True)
        cls.manifest = cls.scratch / "update-manifest.json"
        cls.manifest.write_bytes(b'{"assets":[],"sequence":1,"version":"1.2.0"}\n')
        cls.signature = cls.scratch / "good.sig"
        subprocess.run(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", cls.scratch / "one.pem", "-in", cls.manifest, "-out", cls.signature], check=True, capture_output=True)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.scratch, ignore_errors=True)

    def verify(self, manifest=None, signature=None, key="one"):
        return subprocess.run(["openssl", "pkeyutl", "-verify", "-rawin", "-pubin", "-inkey", self.scratch / f"{key}.pub", "-in", manifest or self.manifest,
                               "-sigfile", signature or self.signature], capture_output=True).returncode == 0

    def test_a_good_signature_verifies_and_every_damaged_one_does_not(self):
        self.assertTrue(self.verify())
        good = self.signature.read_bytes()
        self.assertEqual(len(good), 64)
        cases = {"another key": dict(key="two")}
        for name, content in {"empty": b"", "one byte short": good[:-1], "one byte long": good + b"\0", "a flipped bit": bytes([good[0] ^ 1]) + good[1:],
                              "zeros": bytes(64), "the same twice": good + good, "text": b"signature\n"}.items():
            path = self.scratch / "damaged.sig"
            path.write_bytes(content)
            cases[name] = dict(signature=path)
        for name, arguments in cases.items():
            self.assertFalse(self.verify(**arguments), name)

    def test_every_change_to_the_signed_list_is_detected(self):
        original = self.manifest.read_bytes()
        for name, content in {"a space at the end": original + b" ", "no newline": original[:-1], "another sequence": original.replace(b'"sequence":1', b'"sequence":2'),
                              "another version": original.replace(b"1.2.0", b"1.2.1"), "CRLF": original[:-1] + b"\r\n", "empty": b""}.items():
            path = self.scratch / "changed.json"
            path.write_bytes(content)
            self.assertFalse(self.verify(manifest=path), name)


if __name__ == "__main__":
    unittest.main(verbosity=1)
