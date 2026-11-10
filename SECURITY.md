# Security policy

oneMK sees every key you type and moves your pointer on other computers, so
security reports are treated as the most urgent kind of bug.

## Reporting a vulnerability

Please report privately, not in a public issue:

- **GitHub:** open the repository's **Security** tab and choose **Report a
  vulnerability** (a private advisory only the maintainer sees).
- If that is not possible, open a public issue that says only "security
  contact requested", with no details, and you will be contacted privately.

Include the oneMK version (`one-mk --version`), the operating system and
desktop, and the steps to reproduce. You can expect an acknowledgement
within 7 days and a fix or a plan within 30 days for confirmed issues. You
will be credited in the release notes unless you ask not to be.

## Supported versions

oneMK is young and moves quickly: only the **latest release** gets security
fixes. Updating is the fix: the window offers it (Settings > Updates), or run
`one-mk update`.

| Version | Supported |
| --- | --- |
| Latest release | Yes |
| Anything older | No: update to the latest release |

## What is in scope

- The network link between computers: pairing, the identities and keys it
  creates (`one-mk id`), the encrypted connection, the clipboard and file
  transfers, and anything a computer on the same network can do without
  being paired.
- Local privilege: anything that lets another account or a web page gain
  control through oneMK, including the package update's password step.
- The updater and the release files: anything that would make oneMK install
  code the publisher did not sign.

Out of scope: an attacker who already controls your account (they can run
programs as you anyway), a compromised publisher signing key, and bugs in
the operating system's own input or portal permissions.

## What the update signature protects

Every release carries `update-manifest.json`, an Ed25519-signed list of every
release file with its size and SHA-256 (`update-manifest.json.sig`). The
public key is built into oneMK (`crates/one-mk/src/update_key.rs`); the
private key never leaves the publisher and the release workflow.

- **The in-app updater** (`one-mk update`, Settings > Updates) installs only
  a file listed in a list that verifies against that built-in key, with the
  exact size and SHA-256 the list gives. It refuses older releases, a reused
  release sequence with other contents, revoked releases and downgrades, so
  an old signed release cannot be replayed. GitHub, HTTPS, mirrors and
  `SHA256SUMS` are not trusted on their own.
- **Linux package updates** need your password once, for one root step that
  trusts nothing from your account: it verifies the signed list again with
  the key of the installed oneMK, refuses anything not newer, and installs
  only its own root-only, checked copy of the package (docs/UPDATES.md).
- **macOS app updates** additionally require the new app to be signed with
  the same Apple Developer ID team as the running one.
- **install.sh** checks the download against `SHA256SUMS`, and against the
  signed list wherever OpenSSL 3 is available (most Linux systems). Without
  OpenSSL 3 (macOS ships LibreSSL) the first install relies on GitHub and
  HTTPS; every later update goes through the signed updater.
- **Downloads by hand** (the `.dmg`, `.exe`, `.deb`, AppImage) can be checked
  against `SHA256SUMS`. The Windows installer is not Authenticode-signed yet,
  so Windows SmartScreen warns about it.

What the signature does not protect against: a stolen publisher key (there
is no automatic recovery; a new key needs a bridge release, see
docs/UPDATES.md), a network that hides new releases from you (oneMK never
installs anything older, but it cannot see what it is not shown), and
package managers' own channels (Homebrew, Flatpak), which have their own
trust.
