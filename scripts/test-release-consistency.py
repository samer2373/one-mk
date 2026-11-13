#!/usr/bin/env python3
"""Tests scripts/check-release-consistency.py: it passes on this checkout, and
each of its checks fails on the mistake it exists for.

    python3 scripts/test-release-consistency.py

Works on a scratch copy of the checkout, mutating one file at a time. Needs
Python 3 with PyYAML (CI installs it).
"""
import importlib.util
import io
import contextlib
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
spec = importlib.util.spec_from_file_location("consistency", HERE / "check-release-consistency.py")
consistency = importlib.util.module_from_spec(spec)
spec.loader.exec_module(consistency)

CURRENT = consistency.workspace_version(REPO)
BUMPED = ".".join(CURRENT.split(".")[:2] + [str(int(CURRENT.split(".")[2]) + 1)])
SKIP = shutil.ignore_patterns("target", ".git", "vendor", "screens", "node_modules", "fixtures", "update_fixtures", "*.png", "*.ico", "*.icns", "*.woff2", "*.mp4", "*.webm")


def run(root, *only):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        status = consistency.main(["--root", str(root), *([] if not only else ["--only", *only])])
    return status, out.getvalue()


class Consistency(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scratch = Path(tempfile.mkdtemp(prefix="one-mk-consistency-"))
        cls.copy = cls.scratch / "repo"
        shutil.copytree(REPO, cls.copy, ignore=SKIP)
        # The icon checks look for these files; copy them as empty stand-ins (the checks look only at their names).
        for source in (REPO / "docs/brand/app-icon").rglob("*"):
            if source.is_file():
                target = cls.copy / source.relative_to(REPO)
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(b"")
        cls.saved = {}

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.scratch, ignore_errors=True)

    def tearDown(self):
        for relative, text in self.saved.items():
            (self.copy / relative).write_text(text, encoding="utf-8")
        self.saved.clear()

    def floor(self):
        """install.sh's MIN_SEQUENCE as it is in the copy under test (it changes with every release)."""
        import re
        return int(re.search(r"^MIN_SEQUENCE=([0-9]+)$", (self.copy / "install.sh").read_text(encoding="utf-8"), re.M).group(1))

    def mutate(self, relative, old, new, count=1):
        path = self.copy / relative
        text = path.read_text(encoding="utf-8")
        self.assertIn(old, text, f"{relative} no longer contains the text this test changes: update the test")
        self.saved.setdefault(relative, text)
        path.write_text(text.replace(old, new, count), encoding="utf-8")

    def add_file(self, relative, text):
        path = self.copy / relative
        self.assertFalse(path.exists())
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        self.addCleanup(path.unlink)

    def fails(self, section, expected):
        status, output = run(self.copy, section)
        self.assertEqual(status, 1, f"{section} passed although it should fail with {expected!r}:\n{output}")
        self.assertTrue(any(line.startswith("FAILED") and expected in line for line in output.splitlines()),
                        f"no FAILED line mentions {expected!r}:\n" + "\n".join(l for l in output.splitlines() if l.startswith("FAILED")))

    # ------------------------------------------------------------- baseline
    def test_this_checkout_is_consistent(self):
        status, output = run(self.copy)
        self.assertEqual(status, 0, "\n".join(line for line in output.splitlines() if line.startswith("FAILED")))
        self.assertGreater(output.count("\nok "), 150)

    def test_a_missing_file_is_a_failure_not_a_crash(self):
        (self.copy / "install.sh").rename(self.copy / "install.sh.moved")
        self.addCleanup(lambda: (self.copy / "install.sh.moved").rename(self.copy / "install.sh"))
        status, output = run(self.copy, "artifacts")
        self.assertEqual(status, 1)
        self.assertIn("FAILED  artifacts: FileNotFoundError", output)

    # ------------------------------------------------------------- versions
    def test_a_version_bump_that_misses_the_lock_notes_and_docs_fails(self):
        self.mutate("Cargo.toml", f'version = "{CURRENT}"', f'version = "{BUMPED}"')
        self.fails("versions", f"Cargo.lock holds every workspace crate at {BUMPED}")
        self.fails("versions", f"docs/release-notes/v{BUMPED}.md exists")
        self.fails("versions", f"docs/COMMANDS.md is generated for {BUMPED}")
        self.fails("versions", f"the bug report template's example version is {BUMPED}")

    def test_a_crate_with_its_own_version_fails(self):
        self.mutate("crates/wall/Cargo.toml", "version.workspace = true", 'version = "0.3.0"')
        self.fails("versions", "crates/wall/Cargo.toml inherits the workspace version")

    def test_release_notes_for_a_future_version_fail(self):
        self.add_file("docs/release-notes/v9.9.9.md", "# oneMK 9.9.9\n")
        self.fails("versions", "no release notes for a version newer than Cargo.toml")

    def test_a_stray_file_in_release_notes_fails(self):
        self.add_file("docs/release-notes/draft.md", "# draft\n")
        self.fails("versions", "every file in docs/release-notes is named")

    def test_a_version_line_before_the_workspace_one_fails(self):
        self.mutate("Cargo.toml", "[workspace]\n", '# version = "0.0.1" stray\nversion = "0.0.1"\n[workspace]\n')
        self.fails("versions", "the first `version")

    def test_a_hard_coded_installer_version_fails(self):
        self.mutate("installer/one-mk.iss", "AppVersion={#AppVersion}", f"AppVersion={CURRENT}")
        self.fails("versions", "the Windows installer takes its version")

    def test_the_wrong_tag_is_refused_by_version_sh(self):
        status, output = run(self.copy, "versions")
        self.assertEqual(status, 0, output)
        self.assertIn(f"scripts/version.sh --check accepts exactly the tag v{CURRENT}", output)

    # ------------------------------------------------------------ workflows
    def test_an_unpinned_third_party_action_fails(self):
        self.mutate(".github/workflows/ci.yml", "      - run: sh scripts/version.sh", "      - uses: someone/else@v1\n      - run: sh scripts/version.sh")
        self.fails("workflows", "someone/else@v1 is pinned")

    def test_a_first_party_action_on_a_branch_fails(self):
        self.mutate(".github/workflows/ci.yml", "actions/checkout@v4", "actions/checkout@main")
        self.fails("workflows", "actions/checkout@main is pinned")

    def test_a_checkout_that_keeps_credentials_fails(self):
        self.mutate(".github/workflows/pages.yml", "        with: { persist-credentials: false }\n", "")
        self.fails("workflows", "pages.yml: checkout in build does not keep the token")

    def test_write_access_outside_publish_fails(self):
        self.mutate(".github/workflows/release.yml", "  test:\n    strategy:", "  test:\n    permissions: { contents: write }\n    strategy:")
        self.fails("workflows", "release.yml: job test may write contents")

    def test_missing_permissions_fail(self):
        self.mutate(".github/workflows/ci.yml", "permissions:\n  contents: read\n\n", "")
        self.fails("workflows", "ci.yml declares its permissions")

    def test_pull_request_target_fails(self):
        self.mutate(".github/workflows/ci.yml", "  pull_request:\n", "  pull_request_target:\n")
        self.fails("workflows", "ci.yml does not run on pull_request_target")

    def test_an_unlocked_cargo_command_fails(self):
        self.mutate(".github/workflows/ci.yml", "cargo test --workspace --locked --no-fail-fast", "cargo test --workspace --no-fail-fast")
        self.fails("workflows", "is not --locked")

    def test_a_secret_on_pull_requests_fails(self):
        self.mutate(".github/workflows/ci.yml", "      - run: sh scripts/version.sh", "      - run: sh scripts/version.sh\n        env: { TOKEN: \"${{ secrets.TOKEN }}\" }")
        self.fails("workflows", "reads a secret")

    def test_attacker_controlled_text_in_a_script_fails(self):
        self.mutate(".github/workflows/ci.yml", "      - run: sh scripts/version.sh", "      - run: echo \"${{ github.event.pull_request.title }}\"")
        self.fails("workflows", "attacker-controlled text")

    def test_invalid_shell_in_a_step_fails(self):
        self.mutate(".github/workflows/ci.yml", "      - run: sh scripts/version.sh", "      - run: if then fi")
        self.fails("workflows", "is not valid shell")

    def test_publishing_without_waiting_for_the_checks_fails(self):
        self.mutate(".github/workflows/release.yml", "needs: [test, release-checks, build]", "needs: [build]")
        self.fails("workflows", "publishing waits for the tests")

    def test_an_unpinned_appimagetool_fails(self):
        self.mutate(".github/workflows/release.yml", "APPIMAGETOOL_SHA256: 46fdd785", "APPIMAGETOOL_SHA: 46fdd785")
        self.fails("workflows", "appimagetool is downloaded at a pinned version")

    # ------------------------------------------------------------ artifacts
    def test_a_download_nobody_builds_fails(self):
        self.mutate("README.md", "`one-mk-x86_64-unknown-linux-gnu.AppImage`", "`one-mk-aarch64-unknown-linux-gnu.AppImage`")
        self.fails("artifacts", "one-mk-aarch64-unknown-linux-gnu.AppImage (named in README.md)")

    def test_a_site_link_to_a_missing_file_fails(self):
        self.mutate("site/index.html", "releases/latest/download/one-mk-x86_64-pc-windows-msvc.zip", "releases/latest/download/one-mk-x86_64-pc-windows-msvc.tar.gz")
        self.fails("artifacts", "one-mk-x86_64-pc-windows-msvc.tar.gz")

    def test_install_sh_asking_for_an_unmade_name_fails(self):
        self.mutate("install.sh", "file=one-mk-$target.tar.gz", "file=one-mk-$target.tgz")
        self.mutate("install.sh", "Linux-x86_64)  target=x86_64-unknown-linux-gnu; file=one-mk-$target.tgz", "Linux-x86_64)  target=x86_64-unknown-linux-gnu; file=one-mk-$target.zip")
        self.fails("artifacts", "one-mk-x86_64-unknown-linux-gnu.zip (named in install.sh)")

    def test_a_release_without_the_dmg_step_fails(self):
        self.mutate("scripts/bundle-mac.sh", 'dmg="${out%.app.zip}.dmg"', 'dmg="$tmp/x.dmg"')
        self.fails("artifacts", "release.yml and the scripts it runs make every kind of release file")

    def test_a_renamed_installer_fails(self):
        self.mutate("installer/one-mk.iss", "OutputBaseFilename=one-mk-x86_64-pc-windows-msvc-setup", "OutputBaseFilename=oneMK-setup")
        self.fails("artifacts", "make every kind of release file")

    def test_a_cask_with_other_file_names_fails(self):
        self.mutate("packaging/homebrew/one-mk.rb.in", "one-mk-#{arch}-apple-darwin.dmg", "oneMK-#{arch}.dmg")
        self.fails("artifacts", "the Homebrew cask's file name pattern")

    def test_dropping_a_build_target_breaks_what_names_it(self):
        self.mutate(".github/workflows/release.yml", "          - { os: macos-15,        target: x86_64-apple-darwin }\n", "")
        self.fails("artifacts", "one-mk-x86_64-apple-darwin.dmg")

    # ------------------------------------------------------------- metadata
    def test_a_desktop_file_without_the_final_semicolon_fails(self):
        self.mutate("packaging/linux/one-mk.desktop", "Categories=Utility;RemoteAccess;", "Categories=Utility;RemoteAccess")
        self.fails("metadata", "Categories ends with a semicolon")

    def test_a_desktop_file_running_an_unshipped_program_fails(self):
        self.mutate("packaging/linux/one-mk.desktop", "Exec=one-mk-app", "Exec=one-mk-gui")
        self.fails("metadata", "Exec runs a program the packages ship")

    def test_a_desktop_file_with_a_missing_icon_fails(self):
        self.mutate("packaging/linux/one-mk.desktop", "Icon=one-mk", "Icon=onemk")
        self.fails("metadata", "Icon=onemk is shipped")

    def test_the_deb_menu_entry_drifting_from_the_appimages_fails(self):
        self.mutate("scripts/bundle-deb.sh", "Exec=one-mk-app\nIcon=one-mk\nCategories=Utility;", "Exec=one-mk-tray\nIcon=one-mk\nCategories=Utility;")
        self.fails("metadata", "the .deb's Exec equals")

    def test_a_metainfo_with_another_app_id_fails(self):
        self.mutate("packaging/linux/io.github.onemk.OneMk.metainfo.xml", "<id>io.github.onemk.OneMk</id>", "<id>org.example.OneMk</id>")
        self.fails("metadata", "metainfo id is io.github.onemk.OneMk")

    def test_a_metainfo_launching_a_missing_entry_fails(self):
        self.mutate("packaging/linux/io.github.onemk.OneMk.metainfo.xml", "one-mk.desktop</launchable>", "onemk.desktop</launchable>")
        self.fails("metadata", "metainfo launchable")

    def test_broken_metainfo_xml_fails(self):
        self.mutate("packaging/linux/io.github.onemk.OneMk.metainfo.xml", "<name>oneMK</name>", "<name>oneMK</nam>")
        self.fails("metadata", "metainfo template is well-formed XML")

    def test_a_polkit_action_that_remembers_the_password_fails(self):
        self.mutate("packaging/linux/io.github.onemk.OneMk.update.policy", "<allow_active>auth_admin</allow_active>", "<allow_active>auth_admin_keep</allow_active>")
        self.fails("metadata", "administrator password asked every time")

    def test_a_polkit_action_for_another_program_fails(self):
        self.mutate("packaging/linux/io.github.onemk.OneMk.update.policy", ">/usr/bin/one-mk<", ">/usr/bin/env<")
        self.fails("metadata", "the action covers only /usr/bin/one-mk")

    def test_an_installer_that_needs_administrator_rights_fails(self):
        self.mutate("installer/one-mk.iss", "PrivilegesRequired=lowest", "PrivilegesRequired=admin")
        self.fails("metadata", "installs for the user, no administrator rights")

    def test_an_installer_naming_a_missing_file_fails(self):
        self.mutate("installer/one-mk.iss", '"..\\LICENSE-MIT"', '"..\\LICENCE-MIT"')
        self.fails("metadata", "installer source")

    def test_an_installer_that_leaves_the_path_entry_behind_fails(self):
        self.mutate("installer/one-mk.iss", "  RemoveAppFromPath;\nend;", "end;")
        self.fails("metadata", "installer: uninstalling takes the folder out of PATH")

    def test_an_installer_that_does_not_run_one_mk_uninstall_fails(self):
        self.mutate("installer/one-mk.iss", "Exec(Exe, 'uninstall',", "Exec(Exe, 'integrate --remove',")
        self.fails("metadata", "installer: uninstalling runs `one-mk uninstall`")

    def test_an_installer_that_does_not_stop_the_daemon_fails(self):
        self.mutate("installer/one-mk.iss", "Exec(Exe, 'stop',", "Exec(Exe, 'status',")
        self.fails("metadata", "installer: uninstalling stops the daemon")

    def test_an_installer_that_appends_to_the_path_blindly_fails(self):
        self.mutate("installer/one-mk.iss", "[Code]", '[Registry]\nRoot: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"\n\n[Code]')
        self.fails("metadata", "installer: PATH is changed by whole entries")

    def test_an_installer_removing_another_startup_entry_fails(self):
        self.mutate("installer/one-mk.iss", "RunKey = 'Software\\Microsoft\\Windows\\CurrentVersion\\Run'", "RunKey = 'Software\\Microsoft\\Windows\\CurrentVersion\\RunOnce'")
        self.fails("metadata", "installer: removes the same startup entry")

    def test_an_install_script_without_a_replay_floor_fails(self):
        self.mutate("install.sh", f"MIN_SEQUENCE={self.floor()}\n", "MIN_SEQUENCE=soon\n")
        self.fails("metadata", "install.sh: one MIN_SEQUENCE")

    def test_an_install_script_without_a_stale_age_fails(self):
        self.mutate("install.sh", "STALE_DAYS=180\n", "STALE_DAYS=soon\n")
        self.fails("metadata", "install.sh: one STALE_DAYS")

    def test_dropping_the_pull_request_floor_check_from_ci_fails(self):
        self.mutate(".github/workflows/ci.yml", '--only versions --base "origin/$BASE_REF"', '--only versions')
        self.fails("metadata", "ci.yml checks install.sh's MIN_SEQUENCE against the pull request's base")

    def test_a_replay_floor_above_the_release_being_published_fails(self):
        import os
        floor = self.floor()
        os.environ.update(GITHUB_WORKFLOW="release", GITHUB_RUN_NUMBER=str(floor - 1))
        self.addCleanup(lambda: [os.environ.pop(name) for name in ("GITHUB_WORKFLOW", "GITHUB_RUN_NUMBER")])
        self.fails("metadata", f"MIN_SEQUENCE {floor} is not above this release's sequence {floor - 1}")

    def test_a_release_that_does_not_number_its_update_list_by_the_run_fails(self):
        self.mutate(".github/workflows/release.yml", 'make-update-manifest.sh release-files "$GITHUB_RUN_NUMBER"', 'make-update-manifest.sh release-files 1')
        self.fails("metadata", "release.yml gives the update list the run number")

    def test_an_installer_missing_a_program_fails(self):
        self.mutate("installer/one-mk.iss", 'Source: "{#ReleaseDir}\\one-mk-wallpaper.exe"; DestDir: "{app}"\n', "")
        self.fails("metadata", "installer ships exactly the four programs")

    def test_a_cask_that_stops_the_wrong_launch_agents_fails(self):
        self.mutate("packaging/homebrew/one-mk.rb.in", '"com.one-mk.daemon"', '"com.one-mk.service"')
        self.fails("metadata", "cask uninstall stops the LaunchAgents")

    def test_a_cask_quitting_another_bundle_fails(self):
        self.mutate("packaging/homebrew/one-mk.rb.in", 'quit:      "com.one-mk.app"', 'quit:      "org.example.app"')
        self.fails("metadata", "cask quits the bundle id")

    def test_an_unfilled_template_placeholder_fails(self):
        self.mutate("packaging/flatpak/io.github.onemk.OneMk.yml.in", "sha256: @SHA256_ICON_PNG@", "sha256: @SHA256_ICON_PNG@\n        x: @SHA256_NEW@")
        self.fails("metadata", "package-flatpak.sh fills every placeholder")

    def test_a_flatpak_with_the_whole_home_fails(self):
        self.mutate("packaging/flatpak/io.github.onemk.OneMk.yml.in", "  - --filesystem=xdg-download", "  - --filesystem=home")
        self.fails("metadata", "flatpak asks for no broad access")

    def test_a_flatpak_with_another_app_id_fails(self):
        self.mutate("packaging/flatpak/io.github.onemk.OneMk.yml.in", "app-id: io.github.onemk.OneMk", "app-id: org.example.OneMk")
        self.fails("metadata", "flatpak app-id")

    def test_an_extension_folder_not_named_for_its_uuid_fails(self):
        self.mutate("extensions/gnome/one-mk-pointer@one-mk.org/metadata.json", '"uuid": "one-mk-pointer@one-mk.org"', '"uuid": "pointer@one-mk.org"')
        self.fails("metadata", "one-mk-pointer@one-mk.org: uuid equals its folder name")

    def test_an_extension_version_that_is_not_a_whole_number_fails(self):
        self.mutate("extensions/gnome/one-mk-wallpaper@one-mk.org/metadata.json", '"version": 2', '"version": "2.0"')
        self.fails("metadata", "version is a whole number")

    def test_a_release_note_that_breaks_the_metainfo_fails(self):
        # A note that makes the renderer emit text outside XML rules (a control character).
        self.mutate("docs/release-notes/v1.2.0.md", "# oneMK 1.2.0", "# oneMK 1.2.0\n\nbad \x01 note")
        self.fails("metadata", "release notes v1.2.0.md render to well-formed AppStream XML")


class InstallFloorAgainstBase(unittest.TestCase):
    """--base: install.sh's MIN_SEQUENCE must not go down, and must go up in a pull request that changes the version."""

    def repo(self, floor, version):
        root = Path(tempfile.mkdtemp(prefix="one-mk-floor-"))
        self.addCleanup(shutil.rmtree, root, True)
        subprocess.run(["git", "init", "-q", str(root)], check=True)
        self.write(root, floor, version)
        env = {"GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t", "PATH": os.environ["PATH"], "HOME": str(root)}
        subprocess.run(["git", "-C", str(root), "add", "-A"], check=True, env=env)
        subprocess.run(["git", "-C", str(root), "commit", "-q", "-m", "base"], check=True, env=env)
        subprocess.run(["git", "-C", str(root), "branch", "-q", "base"], check=True, env=env)
        return root

    @staticmethod
    def write(root, floor, version):
        (root / "install.sh").write_text(f"#!/bin/sh\nMIN_SEQUENCE={floor}\n")
        (root / "Cargo.toml").write_text(f'[workspace]\n[workspace.package]\nversion = "{version}"\nedition = "2021"\n')

    def check(self, root):
        out = io.StringIO()
        report = consistency.Report()
        with contextlib.redirect_stdout(out):
            consistency.install_floor_against_base(root, report, "base")
        return report.failures, out.getvalue()

    def test_a_release_that_raises_the_floor_passes(self):
        root = self.repo(48, "1.2.0")
        self.write(root, 55, "1.3.0")
        self.assertEqual(self.check(root)[0], 0)

    def test_a_release_that_leaves_the_floor_where_it_was_fails(self):
        root = self.repo(48, "1.2.0")
        self.write(root, 48, "1.3.0")
        failures, output = self.check(root)
        self.assertEqual(failures, 1, output)
        self.assertIn("this changes the version (1.2.0 to 1.3.0), so MIN_SEQUENCE must be raised past 48", output)

    def test_a_lowered_floor_fails_whatever_the_version(self):
        for version in ("1.2.0", "1.3.0"):
            root = self.repo(48, "1.2.0")
            self.write(root, 40, version)
            failures, output = self.check(root)
            self.assertGreaterEqual(failures, 1, output)
            self.assertIn("MIN_SEQUENCE 40 is not lower than 48", output)

    def test_a_change_that_is_not_a_release_may_keep_the_floor(self):
        root = self.repo(48, "1.2.0")
        self.assertEqual(self.check(root)[0], 0)
        self.write(root, 60, "1.2.0")
        self.assertEqual(self.check(root)[0], 0, "raising it alone (a withdrawn release) is allowed")

    def test_the_change_that_introduces_the_floor_passes_and_losing_it_fails(self):
        root = self.repo(48, "1.2.0")
        (root / "install.sh").write_text("#!/bin/sh\n")  # a base that has no floor yet
        subprocess.run(["git", "-C", str(root), "commit", "-qam", "no floor"], check=True,
                       env={"GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t",
                            "GIT_COMMITTER_EMAIL": "t@t", "PATH": os.environ["PATH"], "HOME": str(root)})
        subprocess.run(["git", "-C", str(root), "branch", "-q", "-f", "base"], check=True)
        self.write(root, 48, "1.2.0")
        failures, output = self.check(root)
        self.assertEqual(failures, 0, output)
        self.assertIn("is new here", output)
        (root / "install.sh").write_text("#!/bin/sh\n")  # neither side has one: that is a failure
        failures, output = self.check(root)
        self.assertEqual(failures, 1, output)

    def test_a_base_that_cannot_be_read_fails_instead_of_passing(self):
        root = self.repo(48, "1.2.0")
        subprocess.run(["git", "-C", str(root), "branch", "-q", "-m", "base", "other"], check=True)
        failures, output = self.check(root)
        self.assertEqual(failures, 1, output)
        self.assertIn("base has install.sh and Cargo.toml to compare with", output)

    def test_the_command_line_runs_it_on_this_checkout(self):
        # --base against HEAD: the same files, so the version is unchanged and the floor is not lower.
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            consistency.main(["--root", str(REPO), "--only", "versions", "--base", "HEAD"])
        self.assertIn("== install floor", out.getvalue())
        self.assertIn("the version is unchanged from HEAD, so MIN_SEQUENCE may stay", out.getvalue())


if __name__ == "__main__":
    sys.exit(unittest.main(verbosity=1, exit=False).result.wasSuccessful() is False)
