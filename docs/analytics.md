# Optional PostHog product analytics

Analytics are **off by default**. Configuration alone does not enable collection.
The old `ONE_MK_ANALYTICS_OPT_IN` environment flag no longer enables analytics.

## Consent and configuration

Use **Settings → General → Privacy → Share anonymous product analytics with
PostHog**, or the CLI (with `--home DIR` for a non-default installation):

```sh
one-mk analytics status
one-mk analytics on
one-mk analytics off
one-mk analytics reset
```

`off` and `reset` both withdraw consent and delete the random ID. Enabling again
creates a new one. Repeated `on` preserves the existing ID. Consent is local to
this settings home, shared between its desktop window, daemon and CLI. Reads
never generate an ID. A missing, incomplete or unreadable consent file fails
closed. Saving a failed preference reports an error; check status and retry.

Each process pins its consent generation before starting work, including an absent consent. After enabling or resetting, restart the desktop app and daemon to begin collection; subsequent CLI invocations use the new generation. Withdrawal immediately prevents new sends (an already in-flight HTTP request cannot be recalled). Existing processes never adopt later consent, so operations begun before consent or before a reset cannot be reported under the new identity. This deliberately favors privacy over seamless opt-in. A failed local withdrawal also disables collection in that process.

An app operator must separately provide these variables to each launched
process, including the desktop app and OS login service:

```sh
export ONE_MK_POSTHOG_KEY=phc_YOUR_PUBLIC_PROJECT_INGEST_KEY
export ONE_MK_POSTHOG_HOST=https://eu.i.posthog.com
```

Use your existing project's region: `https://eu.i.posthog.com` or
`https://us.i.posthog.com`. Other hosts, HTTP, paths and redirects are rejected.
Use a public ingestion key, never a personal API key. No credentials are
included. Missing/invalid configuration sends nothing, even with consent on.
Restart existing processes after configuring their environment.

## Data and coverage

The stable, randomly generated installation ID supports repeat usage/retention
per installation, not cross-device users or accounts. It is not derived from
hardware, paired-device identity or account data. The schema accepts only typed
events, outcome/error enums, app/version, OS family and origin. No user-supplied
properties can be attached. Error categorization uses error types, never text.

Implemented hooks:

| Event | Actual boundary and limitations |
| --- | --- |
| `app_opened` | Desktop link initialization |
| `settings_opened` | First Settings visit per window |
| `daemon_ready` | Daemon state initialized; not proof every OS permission is granted |
| `daemon_finished` | Daemon invocation returned normally or with a handled error; not crash reporting |
| `pairing_finished` | Ordinary pairing trust saved, explicit cancellation/refusal, retry error or startup failure; invitation host/join success after acknowledgment |
| `connection_established` | Daemon peer link connected |
| `connection_closed` | Daemon removes a closed peer; neutral outcome, not necessarily an error |
| `connection_attempt_finished` | Failed dial, fixed `connection` error category; throttled |
| `layout_finished` | Layout command with a mutation returned; read-only queries excluded |
| `profile_finished` | Profile command other than listing returned |
| `transfer_finished` | Outgoing CLI send or desktop drop returned; explicit Ctrl+C is cancellation |
| `update_check_finished` | Explicit check returned, or background check actually ran |
| `update_request_finished` | Update request returned, including already-up-to-date/no installation |
| `update_install_finished` | Installer/apply function returned; separate from request outcome |
| `update_rollback_finished` | Rollback command returned |
| `feature_setting_finished` | Gaming, appearance, lock-sync, edge or pointer-overlay command returned; some support inspection as well as mutation |

GUI actions that invoke CLI commands produce those command events, not duplicate
GUI completion events. Their origin is `cli`; app-open/settings events are
`desktop`, internal daemon connection/background events are `daemon`.

**Not yet covered:** invitation refusal/expiry error taxonomy, incoming transfer
outcomes, transfer byte/file counts, individual feature/page funnels beyond
Settings, automatic profile switches, clipboard functionality, update-download
milestones, native signal/OS crashes and raw exceptions/stacks. Rust panics are separately covered as below. Successful apply means the
installer returned, not that the updated process subsequently started.

No keyboard/mouse activity, clipboard contents, user filenames/paths, peer names,
fingerprints, IP addresses, SSIDs, window titles, screenshots, recordings or
logs are included. There is no autocapture, replay or person profile creation.
GeoIP enrichment is disabled; PostHog still receives the network source IP at
the transport layer. Previously ingested data must be deleted in PostHog.

## Delivery and withdrawal

Delivery uses a maintained `ureq` HTTPS client off the UI/input/runtime thread.
US/EU cloud hosts only; redirects disabled; three-second request budget and
16 KiB response limit. Status/body validation distinguishes accepted, rejected,
quota-limited, throttled and transient failure. Acceptance is not evidence that
an event or grouped issue is visible in PostHog.

The in-memory queue is 32 items, with 60 events/minute/process and one of each
event/outcome class per second (a success does not suppress the first failure). A private atomic spool retains up to 256 events / 512 KiB,
24-hour TTL, at most five attempts with jittered backoff. UUID and original
timestamp stay fixed across retries. Records bind to the consent generation
and hashed destination; changing project or revoking/re-enabling consent cannot
resurrect old data. Permanent rejections and quota-limited events are dropped.
No key is stored in the spool. Opt-out removes retry and panic records.
Requests already in flight cannot be recalled. Exit waits at most 300 ms for
persistence; abrupt termination, disk/permission failure or capacity limits can
still lose telemetry. This is not an exact audit log.

## Redacted errors and Rust panics

Failed typed operations emit both their outcome and an official `$exception`
event with a constant redacted description, category, operation, build/platform,
mechanism and stable safe fingerprint. No error string, cause chain, stack dump,
input or content is serialized. Diagnostic consent is v2: old v1 consent does
not silently authorize this expanded scope.

The Rust panic hook writes a fixed-size marker to a pre-opened private file,
with no payload inspection, heap formatting, blocking mutex or network. It can
retain a numeric source-site ID, line/column and crash timestamp; only a small
compiled allowlist of app-source filenames can become a frame. Unknown sites
have no frame. The original panic hook/abort behavior is preserved. On a later
configured launch, matching-consent/destination markers become redacted exception
events. Crash-time version and timestamp are retained. A changed app build or
source-map manifest rejects the marker rather than attributing it to new code. Markers are bounded (32 armed processes) and expire; clean CLI/window
exit removes its marker. A competing startup may prune a marker armed more than
24 hours earlier, so very long-lived processes are not guaranteed coverage.

This does NOT capture SIGKILL, power loss, native faults/minidumps, every OOM or
all crashes. Disk writes can fail or stall; panic hooks are best effort. No raw
crash report is collected. Windows/Linux runtime and native crash behavior need
separate platform validation. No universal-crash claim is made.

## Verification

Tests use temporary homes and synthetic credentials, with no real event traffic:

```sh
cargo test --locked -p one-mk-ctl analytics
cargo test --locked -p one-mk analytics
cargo clippy --locked -p one-mk -p one-mk-app -p one-mk-ctl --all-targets -- -D warnings
python3 scripts/third-party-notices.py --check
```

Validate real UI layouts and end-to-end ingestion with an explicitly opted-in
test installation and test PostHog project before release. Nothing in this PR
creates a project, activates production, deploys or merges the change.

## Synthetic end-to-end verification (not yet run)

After an explicitly approved project is selected, set its public ingest key and
US/EU host in the environment, then run:

```sh
cargo run --locked -p one-mk-ctl --example verify_posthog -- --confirm-synthetic
```

This creates a throwaway home/identity, uses the actual adapter for one synthetic
success and one safe exception (both marked `synthetic_verification=true`),
prints UUIDs/transport results, and clears local
consent afterward. It never reads the user's normal oneMK home. Query both UUIDs
and the grouped exception in that exact project and retain the query evidence.
No live verification has been performed; this command is not run by unit tests.
Symbol/source uploads require a separate explicit decision.
