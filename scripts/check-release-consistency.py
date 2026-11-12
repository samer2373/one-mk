#!/usr/bin/env python3
"""Fails when what oneMK releases has drifted apart. Static, offline, no build.

    python3 scripts/check-release-consistency.py [--root DIR] [--only SECTION ...] [--base REF]

Sections:
  versions   one version everywhere: Cargo.toml (the single source), every crate,
             Cargo.lock, the release notes, the AppStream metainfo the packages
             carry, the docs that name it, and the version readers the packaging
             scripts use (they must agree with scripts/version.sh).
  workflows  .github/workflows: least-privilege permissions, third-party actions
             pinned to a commit, checkout without stored credentials, no secrets
             on pull requests, `--locked` on every cargo command, valid shell.
  artifacts  every release file that install.sh, the website, the README, the
             docs and the package templates name is one release.yml (or the
             scripts it runs) really produces, for a target it really builds.
  metadata   the .desktop files, the AppStream metainfo (for every release notes
             file), the polkit action, the Inno Setup script, the Homebrew and
             Flatpak templates, the GNOME extension manifests: well formed, and
             consistent with each other and with the code they name.

With --base REF (CI gives a pull request's base branch) it also checks that install.sh's MIN_SEQUENCE
never goes down and goes up in a pull request that changes the version.

Exit status 1 when anything failed. scripts/test-release-consistency.py checks
that each check here really fails on the mistake it is for.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

SEMVER = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
PROGRAMS = ["one-mk", "one-mk-tray", "one-mk-app", "one-mk-wallpaper"]
TARGET_OS = {"apple-darwin": "macos", "pc-windows-msvc": "windows", "unknown-linux-gnu": "linux"}
EXTENSIONS_BY_OS = {
    "macos": [".dmg", ".app.zip", ".tar.gz", ".bin"],
    "windows": [".zip", ".exe", "-setup.exe"],
    "linux": [".tar.gz", ".bin", ".deb", ".AppImage"],
}
TARGET = r"(?:aarch64|x86_64)-(?:apple-darwin|pc-windows-msvc|unknown-linux-gnu)"
RELEASE_FILE = re.compile(
    r"\bone-mk-" + TARGET + r"(?:-setup\.exe|\.dmg|\.app\.zip|\.tar\.gz|\.zip|\.deb|\.AppImage|\.bin|\.exe|\.flatpak)"
    r"|\bone-mk-flatpak-sources\.tar\.gz\b"
)
SHA_PINNED = re.compile(r"@[0-9a-f]{40}$")
FIRST_PARTY_TAG = re.compile(r"^actions/[A-Za-z0-9_.-]+@v[0-9]+$")


class Report:
    def __init__(self):
        self.failures = 0

    def ok(self, text):
        print(f"ok      {text}")

    def bad(self, text):
        print(f"FAILED  {text}")
        self.failures += 1

    def check(self, condition, text):
        (self.ok if condition else self.bad)(text)
        return condition


def read(root, relative):
    return (root / relative).read_text(encoding="utf-8")


def exists(root, relative):
    return (root / relative).is_file()


def sections_of(toml):
    """{section header: [lines]} of a TOML file, enough for Cargo manifests."""
    result, current = {"": []}, ""
    for line in toml.splitlines():
        header = re.match(r"^\s*(\[\[?[^\]]+\]\]?)\s*(#.*)?$", line)
        if header:
            current = header.group(1)
            result.setdefault(current, [])
        else:
            result[current].append(line)
    return result


def version_in(cargo_toml):
    for line in sections_of(cargo_toml).get("[workspace.package]", []):
        match = re.match(r'^\s*version\s*=\s*"([^"]+)"', line)
        if match:
            return match.group(1)
    return None


def workspace_version(root):
    return version_in(read(root, "Cargo.toml"))


# ---------------------------------------------------------------- versions

def versions(root, report):
    version = workspace_version(root)
    if not report.check(version is not None and SEMVER.fullmatch(version), f"Cargo.toml [workspace.package] has a MAJOR.MINOR.PATCH version ({version})"):
        return
    # The packaging scripts read the version with their own sed: first `version = "..."` line.
    first = re.search(r'^version = "(.*)"', read(root, "Cargo.toml"), re.M)
    report.check(first and first.group(1) == version, "the first `version = \"...\"` line of Cargo.toml is the workspace version (bundle-deb.sh, bundle-mac.sh and the release's Windows step read it that way)")
    for script in ["scripts/bundle-deb.sh", "scripts/bundle-mac.sh"]:
        report.check("sed -n 's/^version = \"\\(.*\\)\"/\\1/p' Cargo.toml | head -1" in read(root, script), f"{script} reads the version from Cargo.toml")
    try:
        got = subprocess.run(["sh", str(root / "scripts/version.sh")], capture_output=True, text=True, check=True).stdout.strip()
    except (subprocess.CalledProcessError, OSError) as error:
        got = f"error: {error}"
    report.check(got == version, f"scripts/version.sh prints {version}")
    refused = subprocess.run(["sh", str(root / "scripts/version.sh"), "--check", f"v{version}x"], capture_output=True, text=True)
    accepted = subprocess.run(["sh", str(root / "scripts/version.sh"), "--check", f"v{version}"], capture_output=True, text=True)
    report.check(accepted.returncode == 0 and refused.returncode != 0, "scripts/version.sh --check accepts exactly the tag v" + version)

    # Every crate inherits the workspace version.
    names = {}
    for manifest in sorted((root / "crates").glob("*/Cargo.toml")):
        package = sections_of(manifest.read_text(encoding="utf-8")).get("[package]", [])
        text = "\n".join(package)
        name = re.search(r'^\s*name\s*=\s*"([^"]+)"', text, re.M)
        inherits = re.search(r"^\s*version\.workspace\s*=\s*true", text, re.M) or re.search(r"^\s*version\s*=\s*\{\s*workspace\s*=\s*true\s*\}", text, re.M)
        literal = re.search(r'^\s*version\s*=\s*"', text, re.M)
        report.check(bool(name) and bool(inherits) and not literal, f"{manifest.relative_to(root)} inherits the workspace version")
        if name:
            names[name.group(1)] = manifest
    # Cargo.lock: a stale lock (a crate left on an old version) breaks `--locked` builds.
    lock = read(root, "Cargo.lock")
    stale = []
    for block in lock.split("[[package]]")[1:]:
        name = re.search(r'^name = "([^"]+)"', block, re.M)
        ver = re.search(r'^version = "([^"]+)"', block, re.M)
        if name and ver and name.group(1) in names and "source = " not in block and ver.group(1) != version:
            stale.append(f"{name.group(1)} {ver.group(1)}")
    report.check(not stale, "Cargo.lock holds every workspace crate at " + version + (f" (stale: {', '.join(stale)})" if stale else ""))

    # Release notes: this release has them, no future release has.
    notes = root / "docs" / "release-notes"
    current = notes / f"v{version}.md"
    report.check(current.is_file() and read(root, f"docs/release-notes/v{version}.md").startswith(f"# oneMK {version}"), f"docs/release-notes/v{version}.md exists and is titled \"oneMK {version}\"")
    future = [p.name for p in notes.glob("v*.md") if (m := SEMVER.fullmatch(p.stem[1:])) and tuple(map(int, m.groups())) > tuple(map(int, SEMVER.fullmatch(version).groups()))]
    report.check(not future, "no release notes for a version newer than Cargo.toml" + (f" ({', '.join(future)})" if future else ""))
    unnamed = [p.name for p in notes.glob("*.md") if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+\.md", p.name)]
    report.check(not unnamed, "every file in docs/release-notes is named v<version>.md" + (f" ({', '.join(unnamed)})" if unnamed else ""))

    # Docs and templates that name the current version.
    commands = re.search(r"generated from the built program .*? for (\d+\.\d+\.\d+)", read(root, "docs/COMMANDS.md"), re.S)
    report.check(commands and commands.group(1) == version, f"docs/COMMANDS.md is generated for {version}")
    template = re.search(r"placeholder: one-mk (\d+\.\d+\.\d+)", read(root, ".github/ISSUE_TEMPLATE/bug_report.yml"))
    report.check(template and template.group(1) == version, f"the bug report template's example version is {version}")
    # Packaging never hard-codes the version: it is passed in.
    report.check("AppVersion={#AppVersion}" in read(root, "installer/one-mk.iss") and not re.search(r"^AppVersion=\d", read(root, "installer/one-mk.iss"), re.M), "the Windows installer takes its version from /DAppVersion")
    for template_file in ["packaging/homebrew/one-mk.rb.in", "packaging/homebrew/one-mk-source.rb.in"]:
        text = read(root, template_file)
        report.check("@VERSION@" in text and not re.search(r'version "[0-9]', text), f"{template_file} is filled in with the version, not holding one")
    # The AppStream metainfo the three Linux packages carry, rendered for this version.
    rendered = subprocess.run([sys.executable, "-I", str(root / "scripts/metainfo.py"), version, "2026-01-02"], capture_output=True, text=True)
    report.check(rendered.returncode == 0 and f'<release version="{version}" date="2026-01-02">' in rendered.stdout, f"scripts/metainfo.py renders release {version}")
    # The website carries a stamped placeholder; the stamping step has to be there.
    pages = read(root, ".github/workflows/pages.yml")
    report.check("sh site/tools/stamp.sh _site \"$(sh scripts/version.sh)\"" in pages, "the website is stamped with scripts/version.sh's version when published")


# --------------------------------------------------------------- workflows

def load_yaml(text):
    import yaml
    return yaml.safe_load(text)


def workflow_files(root):
    return sorted((root / ".github" / "workflows").glob("*.yml"))


def iter_steps(workflow):
    for job_name, job in (workflow.get("jobs") or {}).items():
        for step in job.get("steps", []) or []:
            yield job_name, job, step


def workflows(root, report):
    allowed_write = {("release.yml", "publish"): {"contents"}, ("pages.yml", None): {"pages", "id-token"}}
    for path in workflow_files(root):
        name = path.name
        workflow = load_yaml(path.read_text(encoding="utf-8"))
        triggers = workflow.get("on", workflow.get(True)) or {}
        triggers = triggers if isinstance(triggers, dict) else {key: None for key in ([triggers] if isinstance(triggers, str) else triggers)}
        report.check(not ({"pull_request_target", "workflow_run"} & set(triggers)), f"{name} does not run on pull_request_target or workflow_run (they hand secrets to fork code)")
        top = workflow.get("permissions")
        report.check(isinstance(top, dict) and top, f"{name} declares its permissions (no default token scope)")
        top = top if isinstance(top, dict) else {}
        for scope, level in top.items():
            if level == "write":
                report.check(scope in allowed_write.get((name, None), set()), f"{name}: the workflow-wide permission {scope}: write is expected here")
        for job_name, job in (workflow.get("jobs") or {}).items():
            for scope, level in (job.get("permissions") or {}).items():
                if level == "write":
                    report.check(scope in allowed_write.get((name, job_name), set()), f"{name}: job {job_name} may write {scope}")
        for job_name, job, step in iter_steps(workflow):
            uses = step.get("uses")
            if uses and not uses.startswith(("./", "docker://")):
                pinned = SHA_PINNED.search(uses) or FIRST_PARTY_TAG.match(uses)
                report.check(pinned, f"{name}: {uses} is pinned to a commit (or is an actions/* action on a major tag)")
                if uses.startswith("actions/checkout@"):
                    report.check((step.get("with") or {}).get("persist-credentials") is False, f"{name}: checkout in {job_name} does not keep the token in .git/config")
            run = step.get("run")
            if run:
                for line in run.splitlines():
                    stripped = line.strip()
                    if re.match(r"^(sudo\s+)?cargo\s+(build|test|clippy|run|tree|metadata|deny|install)\b", stripped) and "--locked" not in stripped:
                        report.bad(f"{name}: `{stripped}` is not --locked (it could pick newer, unreviewed dependencies)")
                if re.search(r"\$\{\{\s*github\.(head_ref|event\.(pull_request|issue|comment|review|head_commit)[^}]*)\}\}", run):
                    report.bad(f"{name}: step {step.get('name', job_name)} puts attacker-controlled text into a shell script")
                if step.get("shell") in (None, "bash", "sh"):
                    probe = re.sub(r"\$\{\{.*?\}\}", "EXPRESSION", run)
                    result = subprocess.run(["bash", "-n"], input=probe, text=True, capture_output=True)
                    if result.returncode != 0:
                        report.bad(f"{name}: a run block of {job_name} is not valid shell: {result.stderr.strip()}")
            if "secrets." in str(step.get("env", "")) + str(step.get("with", "")) + str(run or "") and name == "ci.yml":
                report.bad(f"ci.yml: {job_name} reads a secret, and ci.yml runs on pull requests")
    release = load_yaml(read(root, ".github/workflows/release.yml"))
    jobs = release["jobs"]
    publish = jobs["publish"]
    report.check(set(publish.get("needs", [])) >= {"test", "release-checks", "build"}, "release.yml: publishing waits for the tests, the release checks and every build")
    report.check("refs/tags/" in str(publish.get("if", "")), "release.yml: only a tag publishes")
    text = read(root, ".github/workflows/release.yml")
    report.check(re.search(r"APPIMAGETOOL_SHA256:\s*[0-9a-f]{64}", text) and 'sha256sum -c -' in text, "release.yml: appimagetool is downloaded at a pinned version and checked against its SHA-256")
    report.check("dmgbuild==" in text, "release.yml: dmgbuild is pinned to a version")
    report.check(re.search(r"cargo-deny@\d+\.\d+\.\d+", text) is not None, "release.yml: cargo-deny is pinned to a version")
    report.check("sh scripts/version.sh --check" in text, "release.yml: the tag has to match the Cargo.toml version")
    report.check("Refusing to overwrite a published release" in text and "--draft" in text, "release.yml: a published release is never overwritten (draft first)")
    for needed in ["scripts/check-release-consistency.py"]:
        ci = read(root, ".github/workflows/ci.yml")
        report.check(needed in ci, f"ci.yml runs {needed}")


# --------------------------------------------------------------- artifacts

def matrix_targets(release):
    targets = []
    for include in release["jobs"]["build"]["strategy"]["matrix"]["include"]:
        targets.append(include["target"])
    return targets


def produced_files(root, report):
    """The release's file names, each only if release.yml (or the script it runs) has the step that makes it."""
    release = read(root, ".github/workflows/release.yml")
    targets = matrix_targets(load_yaml(release))
    evidence = {
        ".tar.gz": '"$name.tar.gz"' in release or "tar czf" in release,
        ".zip": '"$name.zip"' in release,
        ".bin": 'one-mk-${{ matrix.target }}.bin' in release,
        ".exe": 'one-mk-${{ matrix.target }}.exe' in release,
        ".app.zip": 'one-mk-${{ matrix.target }}.app.zip' in release,
        ".dmg": '${out%.app.zip}.dmg' in read(root, "scripts/bundle-mac.sh"),
        "-setup.exe": "OutputBaseFilename=one-mk-x86_64-pc-windows-msvc-setup" in read(root, "installer/one-mk.iss")
                      and './one-mk-${{ matrix.target }}-setup.exe' in release,
        ".deb": 'one-mk-${{ matrix.target }}.deb' in release,
        ".AppImage": 'one-mk-${{ matrix.target }}.AppImage' in release,
    }
    produced = set()
    for target in targets:
        operating_system = next(os_name for key, os_name in TARGET_OS.items() if target.endswith(key))
        for extension in EXTENSIONS_BY_OS[operating_system]:
            if evidence.get(extension):
                produced.add(f"one-mk-{target}{extension}")
    report.check(all(evidence.values()), "release.yml and the scripts it runs make every kind of release file" + ("" if all(evidence.values()) else f" (missing: {', '.join(k for k, v in evidence.items() if not v)})"))
    report.check("one-mk-*.*" in release, "release.yml uploads one-mk-*.* from each build")
    # Made by the publish job.
    scripts = read(root, "scripts/package-flatpak.sh") + read(root, "scripts/package-homebrew.sh")
    if "one-mk-flatpak-sources.tar.gz" in scripts:
        produced.add("one-mk-flatpak-sources.tar.gz")
    return produced, targets


def referenced_files(root):
    """{release file name: [places that name it]}."""
    places = {}

    def add(name, where):
        places.setdefault(name, []).append(where)

    sources = ["README.md", ".github/workflows/release.yml", "docs/PACKAGING.md", "docs/TESTING.md", "docs/UPDATES.md", "docs/WINDOWS.md",
               "docs/LIVE-WALLPAPER.md", "site/index.html", "site/404.html", "site/README.md", "site/js/detect.js", "site/js/site.js",
               "packaging/homebrew/one-mk.rb.in", "scripts/package-flatpak.sh", "scripts/package-homebrew.sh"]
    for relative in sources:
        if exists(root, relative):
            for name in RELEASE_FILE.findall(read(root, relative)):
                add(name, relative)
    # install.sh builds its names from the target: one-mk-$target.app.zip / .tar.gz.
    install = read(root, "install.sh")
    for line in install.splitlines():
        found = re.search(r"target=(" + TARGET + r"); file=one-mk-\$target(\.[A-Za-z.]+)", line)
        if found:
            add(f"one-mk-{found.group(1)}{found.group(2)}", "install.sh")
    return places


def artifacts(root, report):
    produced, targets = produced_files(root, report)
    places = referenced_files(root)
    report.check(places, f"{len(places)} release file names are referenced from install.sh, the site, the README and the docs")
    for name in sorted(places):
        if name.endswith(".flatpak"):
            # Built locally only when flatpak-builder is present; never uploaded.
            where = [p for p in places[name] if not p.startswith("scripts/")]
            report.check(not where, f"{name} is a local build output and is not advertised as a download (found in {where})" if where else f"{name} is not advertised as a download")
            continue
        report.check(name in produced, f"{name} (named in {', '.join(sorted(set(places[name])))}) is made by release.yml")
    # install.sh must be able to install on everything the release builds for Mac and Linux.
    install = read(root, "install.sh")
    for target in targets:
        if "windows" in target:
            continue
        report.check(f"target={target}" in install, f"install.sh knows the release target {target}")
    # The release notes table, the site and the README name the same Windows installer.
    windows = [t for t in targets if "windows" in t]
    for target in windows:
        setup = f"one-mk-{target}-setup.exe"
        for relative in ["README.md", "site/index.html", ".github/workflows/release.yml"]:
            report.check(setup in read(root, relative), f"{relative} offers {setup}")
    # Names the update manifest classifies: each file the release makes must keep the shape it accepts.
    for name in sorted(produced):
        report.check(re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,159}", name) is not None, f"{name} is a name the update manifest accepts")
    # Every download link on the site points at the release's latest/download folder, by a file that exists.
    for relative in ["site/index.html"]:
        for match in re.finditer(r'href="https://github\.com/samer2373/one-mk/releases/latest/download/([^"]+)"', read(root, relative)):
            file = match.group(1)
            report.check(file in produced or file in {"SHA256SUMS", "update-manifest.json", "update-manifest.json.sig"}, f"{relative}: link to {file} is a release file")
    # The signed list and the checksums are published alongside, under the names install.sh asks for.
    release = read(root, ".github/workflows/release.yml")
    for name in ["SHA256SUMS", "update-manifest.json"]:
        report.check(name in install and name in release + read(root, "scripts/make-update-manifest.sh"), f"{name} is asked for by install.sh and made by the release")
    report.check("update-manifest.json.sig" in install and "update-manifest.json.sig" in read(root, "scripts/sign-update-manifest.sh"), "update-manifest.json.sig is asked for by install.sh and made by sign-update-manifest.sh")
    # The Homebrew cask downloads the dmg names the release publishes.
    cask = read(root, "packaging/homebrew/one-mk.rb.in")
    report.check("one-mk-#{arch}-apple-darwin.dmg" in cask and 'arm: "aarch64", intel: "x86_64"' in cask, "the Homebrew cask's file name pattern matches the release's .dmg files")
    for arch in ["aarch64", "x86_64"]:
        report.check(f"one-mk-{arch}-apple-darwin.dmg" in produced, f"the cask's {arch} .dmg is made by the release")


# ---------------------------------------------------------------- metadata

def parse_desktop(text):
    entries, group = {}, None
    for number, line in enumerate(text.splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        header = re.fullmatch(r"\[([^\[\]]+)\]", line)
        if header:
            group = header.group(1)
            entries[group] = {}
            continue
        key = re.fullmatch(r"([A-Za-z0-9-]+(?:\[[^\]]+\])?)=(.*)", line)
        if not key or group is None:
            raise ValueError(f"line {number}: {line!r} is not a key=value in a group")
        entries[group][key.group(1)] = key.group(2)
    return entries


def desktop_checks(root, report, label, text, icon_names):
    try:
        entries = parse_desktop(text)
    except ValueError as error:
        report.bad(f"{label} parses: {error}")
        return {}
    entry = entries.get("Desktop Entry", {})
    report.check(set(entries) == {"Desktop Entry"}, f"{label} has one [Desktop Entry] group")
    for key in ["Type", "Name", "Exec", "Icon"]:
        report.check(bool(entry.get(key)), f"{label} sets {key}")
    report.check(entry.get("Type") == "Application", f"{label} is Type=Application")
    for key in ["Categories", "Keywords"]:
        if key in entry:
            report.check(entry[key].endswith(";"), f"{label}: {key} ends with a semicolon")
    program = entry.get("Exec", "").split()[0] if entry.get("Exec") else ""
    report.check(program in PROGRAMS, f"{label}: Exec runs a program the packages ship ({program})")
    report.check(not re.search(r"%[^fFuUdDnNickvm%]", entry.get("Exec", "")), f"{label}: Exec holds only valid field codes")
    report.check(entry.get("Icon") in icon_names, f"{label}: Icon={entry.get('Icon')} is shipped in the icon theme folder")
    report.check(entry.get("Terminal", "false") == "false", f"{label} does not open a terminal")
    return entry


def metadata(root, report):
    icons = root / "docs/brand/app-icon/windows-linux/hicolor"
    icon_names = {p.stem for p in icons.glob("*/apps/*.png")} | {p.stem for p in icons.glob("*/apps/*.svg")}
    entry = desktop_checks(root, report, "packaging/linux/one-mk.desktop", read(root, "packaging/linux/one-mk.desktop"), icon_names)
    # The .deb writes its own entry inline: the same program and icon, so the menu looks the same.
    deb = re.search(r"cat > \"\$root/usr/share/applications/one-mk.desktop\" <<DESKTOP\n(.*?)\nDESKTOP", read(root, "scripts/bundle-deb.sh"), re.S)
    report.check(deb is not None, "scripts/bundle-deb.sh writes the menu entry")
    if deb:
        inline = desktop_checks(root, report, "the .deb's one-mk.desktop", deb.group(1), icon_names)
        for key in ["Name", "Exec", "Icon"]:
            report.check(inline.get(key) == entry.get(key), f"the .deb's {key} equals the AppImage's and Flatpak's ({entry.get(key)})")
    for sizes in ["16x16", "32x32", "48x48", "128x128", "256x256"]:
        report.check((icons / sizes / "apps" / "one-mk.png").is_file(), f"icon theme has {sizes}/apps/one-mk.png")
    report.check((icons / "scalable/apps/one-mk.svg").is_file(), "icon theme has scalable/apps/one-mk.svg (the Flatpak ships it)")
    for relative in ["docs/brand/app-icon/windows-linux/onemk.ico", "docs/brand/app-icon/macos/onemk.icns"]:
        report.check(exists(root, relative), f"{relative} exists (the installers embed it)")

    # AppStream metainfo: the template, then every release's rendering of it.
    template = read(root, "packaging/linux/io.github.onemk.OneMk.metainfo.xml")
    try:
        tree = ET.fromstring(template.replace("@VERSION@", "1.0.0").replace("@DATE@", "2026-01-02").replace("@RELEASE_NOTES@", ""))
    except ET.ParseError as error:
        report.bad(f"the metainfo template is well-formed XML: {error}")
        tree = None
    if tree is not None:
        report.check(tree.findtext("id") == "io.github.onemk.OneMk", "metainfo id is io.github.onemk.OneMk")
        report.check(tree.findtext("id") == re.search(r'LINUX_APP_ID: &str = "([^"]+)"', read(root, "crates/proto/src/lib.rs")).group(1), "metainfo id equals the app id the code registers (proto::LINUX_APP_ID)")
        launchable = tree.findtext("launchable")
        report.check(launchable == "one-mk.desktop", "metainfo launchable is the menu entry the packages install (one-mk.desktop)")
        report.check(tree.findtext("project_license") == "MIT OR Apache-2.0" and tree.findtext("metadata_license") == "CC0-1.0", "metainfo licences: project MIT OR Apache-2.0, metadata CC0-1.0")
        report.check(re.search(r'^license = "MIT OR Apache-2.0"', read(root, "Cargo.toml"), re.M) is not None, "Cargo.toml licence equals the metainfo's")
        provided = {e.text for e in tree.findall("provides/binary")}
        report.check(provided <= set(PROGRAMS), f"metainfo provides only programs the packages ship ({sorted(provided)})")
        for field in ["name", "summary"]:
            report.check(bool(tree.findtext(field)), f"metainfo has a {field}")
        report.check(tree.find("content_rating") is not None, "metainfo has a content rating")
        for url in tree.findall("url"):
            report.check(url.text.startswith("https://github.com/samer2373/one-mk"), f"metainfo url {url.get('type')} points at the project")
    # Every release notes file must turn into valid XML (a stray & or < in a note would break all three packages).
    for notes in sorted((root / "docs/release-notes").glob("v*.md")):
        version = notes.stem[1:]
        rendered = subprocess.run([sys.executable, "-I", str(root / "scripts/metainfo.py"), version, "2026-01-02"], capture_output=True, text=True)
        try:
            ET.fromstring(rendered.stdout)
            good = rendered.returncode == 0
        except ET.ParseError:
            good = False
        report.check(good, f"release notes {notes.name} render to well-formed AppStream XML")
    if subprocess.run(["which", "appstreamcli"], capture_output=True).returncode == 0:
        rendered = subprocess.run([sys.executable, "-I", str(root / "scripts/metainfo.py"), workspace_version(root), "2026-01-02"], capture_output=True, text=True)
        scratch = root / ".metainfo-check.xml"
        try:
            scratch.write_text(rendered.stdout, encoding="utf-8")
            result = subprocess.run(["appstreamcli", "validate", "--no-net", str(scratch)], capture_output=True, text=True)
            report.check(result.returncode == 0, "appstreamcli validates the rendered metainfo" + ("" if result.returncode == 0 else f": {result.stdout.strip()}"))
        finally:
            scratch.unlink(missing_ok=True)
    else:
        print("skipped appstreamcli not installed")

    # polkit action: the root step runs only /usr/bin/one-mk, password every time.
    try:
        policy = ET.parse(root / "packaging/linux/io.github.onemk.OneMk.update.policy").getroot()
    except ET.ParseError as error:
        report.bad(f"the polkit policy is well-formed XML: {error}")
        policy = None
    if policy is not None:
        action = policy.find("action")
        report.check(action is not None and action.get("id") == "io.github.onemk.OneMk.update", "polkit action id is io.github.onemk.OneMk.update")
        defaults = {child.tag: child.text for child in action.find("defaults")}
        report.check(defaults == {"allow_any": "auth_admin", "allow_inactive": "auth_admin", "allow_active": "auth_admin"}, "polkit: administrator password asked every time, never remembered")
        annotation = {a.get("key"): a.text for a in action.findall("annotate")}
        report.check(annotation.get("org.freedesktop.policykit.exec.path") == "/usr/bin/one-mk", "polkit: the action covers only /usr/bin/one-mk")
        sources = "".join(p.read_text(encoding="utf-8", errors="ignore") for p in (root / "crates/one-mk/src").glob("update*.rs"))
        report.check("io.github.onemk.OneMk.update" in sources or "pkexec" in sources, "the updater asks pkexec for the root step")

    # Inno Setup.
    iss = read(root, "installer/one-mk.iss")
    report.check(re.search(r"^AppId=\{\{[0-9A-F]{8}(-[0-9A-F]{4}){3}-[0-9A-F]{12}\}", iss, re.M) is not None, "installer: AppId is a GUID (it keys upgrades and uninstalling)")
    report.check("PrivilegesRequired=lowest" in iss, "installer: installs for the user, no administrator rights")
    report.check("ArchitecturesAllowed=x64compatible" in iss, "installer: 64-bit only, like the release")
    sources = re.findall(r'^Source: "([^"]+)"', iss, re.M)
    shipped = {Path(re.sub(r"\{#ReleaseDir\}\\", "", s).replace("\\", "/")).name for s in sources if "{#ReleaseDir}" in s}
    report.check(shipped == {p + ".exe" for p in PROGRAMS}, f"installer ships exactly the four programs ({sorted(shipped)})")
    for source in sources:
        if "{#ReleaseDir}" in source:
            continue
        target = (root / "installer" / source.replace("\\", "/")).resolve()
        report.check(target.is_file(), f"installer source {source} exists")
    for icon in re.findall(r"^SetupIconFile=(.*)$", iss, re.M):
        report.check((root / "installer" / icon.replace("\\", "/")).resolve().is_file(), f"installer icon {icon} exists")
    # Uninstalling (Inno Setup is not available on Linux: these read the script's text, and
    # scripts/test-windows-path.py runs its PATH functions; CI compiles and installs it on Windows).
    step = re.search(r"^procedure CurUninstallStepChanged\(.*?^end;\n", iss, re.M | re.S)
    step = step.group(0) if step else ""
    report.check("usUninstall" in step and "'uninstall'" in step, "installer: uninstalling runs `one-mk uninstall` (login entry, Send-to entries, wallpaper) before any file is removed")
    report.check("'stop'" in step and "EndRunningPrograms(False)" in step, "installer: uninstalling stops the daemon, the tray, the window and the wallpaper helper")
    report.check("RemoveAppFromPath" in step and "RemoveStartAtSignIn" in step, "installer: uninstalling takes the folder out of PATH and the startup entry out of the registry")
    report.check("ChangesEnvironment=yes" in iss, "installer: other programs are told when PATH changes")
    report.check("{olddata}" not in iss and "RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path'" in iss, "installer: PATH is changed by whole entries (no blind append), as an expandable value")
    run_key = re.search(r'const RUN_KEY: &str = r"HKCU\\([^"]+)"', read(root, "crates/ctl/src/install.rs"))
    report.check(run_key is not None and f"RunKey = '{run_key.group(1)}'" in iss and "RegDeleteValue(HKCU, RunKey, 'one-mk')" in iss, "installer: removes the same startup entry `one-mk install` writes on Windows")
    report.check("taskkill" not in iss.lower() and "ExecutablePath.StartsWith(" in iss, "installer: running programs are ended by full path, not by name alone")
    report.check("scripts/test-windows-path.py" in read(root, ".github/workflows/ci.yml"), "ci.yml runs scripts/test-windows-path.py (the installer's PATH logic)")

    # Homebrew cask: labels and bundle id are what the code and the app use.
    cask = read(root, "packaging/homebrew/one-mk.rb.in")
    install_rs = read(root, "crates/ctl/src/install.rs")
    labels = re.findall(r'^const (?:TRAY_)?LABEL: &str = "([^"]+)"', install_rs, re.M)
    uninstall = re.search(r"uninstall launchctl: \[([^\]]*)\]", cask)
    report.check(uninstall and sorted(re.findall(r'"([^"]+)"', uninstall.group(1))) == sorted(labels), f"cask uninstall stops the LaunchAgents the code installs ({labels})")
    bundle_id = re.search(r"<key>CFBundleIdentifier</key><string>([^<]+)</string>", read(root, "scripts/bundle-mac.sh"))
    quit_id = re.search(r'quit:\s*"([^"]+)"', cask)
    report.check(bundle_id and quit_id and bundle_id.group(1) == quit_id.group(1), "cask quits the bundle id the app is built with")
    report.check('app "one-mk.app"' in cask and "Contents/MacOS/one-mk" in cask, "cask installs one-mk.app and links the one-mk program")
    # Homebrew and Flatpak templates: every placeholder is one the script fills.
    for template_file, script in [("packaging/homebrew/one-mk.rb.in", "scripts/package-homebrew.sh"), ("packaging/homebrew/one-mk-source.rb.in", "scripts/package-homebrew.sh"),
                                  ("packaging/flatpak/io.github.onemk.OneMk.yml.in", "scripts/package-flatpak.sh")]:
        placeholders = set(re.findall(r"@([A-Z0-9_]+)@", read(root, template_file)))
        script_text = read(root, script)
        missing = {p for p in placeholders if f"@{p}@" not in script_text}
        report.check(not missing, f"{script} fills every placeholder of {template_file}" + (f" (not filled: {sorted(missing)})" if missing else ""))
    flatpak = read(root, "packaging/flatpak/io.github.onemk.OneMk.yml.in")
    report.check(re.search(r"^app-id: io\.github\.onemk\.OneMk$", flatpak, re.M) is not None, "flatpak app-id is io.github.onemk.OneMk")
    report.check(re.search(r"^command: one-mk-app$", flatpak, re.M) is not None, "flatpak runs one-mk-app")
    report.check(not re.search(r"--filesystem=(host|home)|--socket=(session-bus|system-bus)|--device=all|--talk-name=org\.freedesktop\.(Flatpak|systemd1)", flatpak), "flatpak asks for no broad access (whole home, the buses, every device)")
    report.check("one-mk-wallpaper" in flatpak and "install -Dm755 one-mk one-mk-tray one-mk-app one-mk-wallpaper" in flatpak, "flatpak installs the four programs")
    metainfo_sed = "sed -i 's|one-mk.desktop|${FLATPAK_ID}.desktop|' io.github.onemk.OneMk.metainfo.xml" in flatpak
    report.check(metainfo_sed, "flatpak renames the launchable inside its metainfo to the app-id desktop file")

    # The GNOME extensions: manifest, folder name and what the code embeds.
    for folder in sorted((root / "extensions/gnome").iterdir()):
        manifest_path = folder / "metadata.json"
        try:
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        except (OSError, ValueError) as error:
            report.bad(f"{manifest_path.relative_to(root)} is valid JSON: {error}")
            continue
        report.check(manifest.get("uuid") == folder.name, f"{folder.name}: uuid equals its folder name")
        report.check(isinstance(manifest.get("version"), int) and manifest["version"] >= 1, f"{folder.name}: version is a whole number (extensions.gnome.org rule)")
        report.check(all(re.fullmatch(r"[0-9]+(\.[0-9]+)?", v) for v in manifest.get("shell-version", [])) and manifest.get("shell-version"), f"{folder.name}: shell-version lists GNOME versions")
        report.check(manifest.get("license") == "MIT OR Apache-2.0" and manifest.get("url", "").startswith("https://github.com/samer2373/one-mk"), f"{folder.name}: licence and url")
        for file in ["extension.js", "bridge.xml"]:
            report.check((folder / file).is_file(), f"{folder.name} has {file}")
        if "wallpaper" in folder.name:
            report.check(f"extensions/gnome/{folder.name}/metadata.json" in read(root, "crates/one-mk/src/wallpaper_extension.rs"), f"the program embeds {folder.name}'s files")
            report.check(f"extensions/gnome/{folder.name}" in read(root, "scripts/bundle-deb.sh") or 'extension=one-mk-wallpaper@one-mk.org' in read(root, "scripts/bundle-deb.sh"), f"the .deb ships {folder.name} outside GNOME's own folders")
        try:
            ET.parse(folder / "bridge.xml")
            report.ok(f"{folder.name}: bridge.xml is well-formed D-Bus introspection XML")
        except ET.ParseError as error:
            report.bad(f"{folder.name}: bridge.xml: {error}")
    # Update key: the production flag and the key the installer trusts must be the same file the release signs against.
    key = read(root, "crates/one-mk/src/update_key.rs")
    report.check(re.search(r"PUBLIC_KEY: \[u8; 32\]", key) is not None and re.search(r"PRODUCTION_KEY: bool", key) is not None, "update_key.rs holds the public key and the production flag")

    # install.sh refuses a signed release older than MIN_SEQUENCE (a replay). The sequence is the run
    # number of the release workflow (make-update-manifest.sh is given $GITHUB_RUN_NUMBER), so on a
    # release run the floor must not be above this very release, or install.sh would refuse it.
    floor = re.findall(r"^MIN_SEQUENCE=([0-9]+)$", read(root, "install.sh"), re.M)
    report.check(len(floor) == 1 and 1 <= int(floor[0]) < 2**63, "install.sh: one MIN_SEQUENCE, a positive plain number")
    report.check("GITHUB_RUN_NUMBER" in read(root, ".github/workflows/release.yml") and "make-update-manifest.sh release-files \"$GITHUB_RUN_NUMBER\"" in read(root, ".github/workflows/release.yml"),
                 "release.yml gives the update list the run number as its sequence (the number install.sh's MIN_SEQUENCE is compared with)")
    stale = re.findall(r"^STALE_DAYS=([0-9]+)$", read(root, "install.sh"), re.M)
    report.check(len(stale) == 1 and 1 <= int(stale[0]) <= 3650, "install.sh: one STALE_DAYS, a plain number of days")
    report.check("--base" in read(root, ".github/workflows/ci.yml") and "check-release-consistency.py" in read(root, ".github/workflows/ci.yml"),
                 "ci.yml checks install.sh's MIN_SEQUENCE against the pull request's base (check-release-consistency.py --base)")
    run_number = os.environ.get("GITHUB_RUN_NUMBER", "")
    if os.environ.get("GITHUB_WORKFLOW") == "release" and run_number.isdigit() and floor:
        report.check(int(floor[0]) <= int(run_number), f"install.sh: MIN_SEQUENCE {floor[0]} is not above this release's sequence {run_number}")


def install_floor_against_base(root, report, base):
    """install.sh's MIN_SEQUENCE is what keeps a fresh install from being pinned to an old release (docs/PACKAGING.md,
    "install.sh and replayed releases"). It narrows the replay window only while it follows the releases, so a pull
    request that changes the version (a release) must raise it past what the base branch has, and nothing may lower it."""
    def at_base(path):
        shown = subprocess.run(["git", "-C", str(root), "show", f"{base}:{path}"], capture_output=True, text=True)
        return shown.stdout if shown.returncode == 0 else None
    old_script, old_cargo = at_base("install.sh"), at_base("Cargo.toml")
    if not report.check(old_script is not None and old_cargo is not None, f"{base} has install.sh and Cargo.toml to compare with"):
        return
    floor = r"^MIN_SEQUENCE=([0-9]+)$"
    before, now = re.search(floor, old_script, re.M), re.search(floor, read(root, "install.sh"), re.M)
    if before is None and now is not None:
        # The change that introduces the floor: there is nothing on the base to compare with yet.
        report.ok(f"install.sh: MIN_SEQUENCE {now.group(1)} is new here ({base} has none yet)")
        return
    if not report.check(before is not None and now is not None, f"install.sh has a MIN_SEQUENCE here and on {base}"):
        return
    before, now = int(before.group(1)), int(now.group(1))
    report.check(now >= before, f"install.sh: MIN_SEQUENCE {now} is not lower than {before} on {base}")
    if version_in(old_cargo) != workspace_version(root):
        report.check(now > before, f"install.sh: this changes the version ({version_in(old_cargo)} to {workspace_version(root)}), so MIN_SEQUENCE must be raised past {before} "
                                   "to the previous release's sequence (its update-manifest.json), or a replay of an older release would stay installable")
    else:
        report.ok(f"install.sh: the version is unchanged from {base}, so MIN_SEQUENCE may stay")


SECTIONS = {"versions": versions, "workflows": workflows, "artifacts": artifacts, "metadata": metadata}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--base", metavar="REF", help="also check that install.sh's MIN_SEQUENCE follows the releases, against this git ref (a pull request's base)")
    parser.add_argument("--only", nargs="+", choices=sorted(SECTIONS), default=sorted(SECTIONS, key=list(SECTIONS).index))
    arguments = parser.parse_args(argv)
    report = Report()
    for section in arguments.only:
        print(f"== {section}")
        try:
            SECTIONS[section](arguments.root.resolve(), report)
        except Exception as error:  # a file the check needs is missing or malformed: that is a failure, not a crash
            report.bad(f"{section}: {type(error).__name__}: {error}")
    if arguments.base:
        print("== install floor")
        try:
            install_floor_against_base(arguments.root.resolve(), report, arguments.base)
        except Exception as error:
            report.bad(f"install floor: {type(error).__name__}: {error}")
    print("all release consistency checks passed" if report.failures == 0 else f"{report.failures} release consistency check(s) failed")
    return 1 if report.failures else 0


if __name__ == "__main__":
    sys.exit(main())
