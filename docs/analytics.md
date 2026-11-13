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
this settings home **and exact app/environment/project/host destination**, shared between its desktop window, daemon and CLI. Reads
never generate an ID. A missing, incomplete or unreadable consent file fails
closed. Saving a failed preference reports an error; check status and retry.

Each process pins its consent generation before starting work, including an absent consent. After enabling or resetting, restart the desktop app and daemon to begin collection; subsequent CLI invocations use the new generation. Withdrawal immediately prevents new sends (an already in-flight HTTP request cannot be recalled). Existing processes never adopt later consent, so operations begun before consent or before a reset cannot be reported under the new identity. This deliberately favors privacy over seamless opt-in. A failed local withdrawal also disables collection in that process.

An app operator must separately provide these variables to each launched
process, including the desktop app and OS login service:

```sh
export ONE_MK_POSTHOG_KEY=phc_YOUR_PUBLIC_PROJECT_INGEST_KEY
export ONE_MK_POSTHOG_HOST=https://eu.i.posthog.com
export ONE_MK_POSTHOG_ENVIRONMENT=production
```

Use your existing project's region: `https://eu.i.posthog.com` or
`https://us.i.posthog.com`. Other hosts, HTTP, paths and redirects are rejected.
Use a public ingestion key, never a personal API key. No credentials are
included. Missing/invalid configuration sends nothing, even with consent on.
Restart existing processes after configuring their environment.

For local development, copy `.env.posthog.example` to the git-ignored
`.env.posthog.local` and supply the approved public capture key and regional
host. A development-only local configuration was prepared during verification;
it contains no synthetic-test flag and does not grant consent. Load it explicitly:

```sh
. ./.env.posthog.local
cargo run --locked -p one-mk -- --home /path/to/your/chosen-home analytics status
```

Do not commit that file or pass the public key through command-line arguments.
GUI apps/login services do not inherit terminal exports automatically; configuring
those launch environments is separate and was not performed here.

`ONE_MK_POSTHOG_ENVIRONMENT` (or generic `POSTHOG_ENVIRONMENT`) accepts only
`development`, `staging`, `production`, or `verification`. Invalid values fail
closed. Unset values default to `development` in debug builds and `production`
in release builds. Set it explicitly for services. Consent v4 has separate files
per environment; prior v1/v2/v3 consent does not enable this schema. Changing the
environment requires its own opt-in and restart. Destinations, retry records and
panic markers bind to that environment as well as the project. Withdrawal
clears retry/panic storage conservatively for the settings home.

Every payload has `app_id=one_mk`, `environment`, and `schema_version=2`;
legacy `app=onemk` is retained. The wire distinct ID is
`one_mk:<environment>:<random installation ID>`, never an account or device ID.
Error fingerprints include the same app/environment scope. A shared project
provides filtered views, **not independent data-access boundaries**. Production
queries must filter both `app_id=one_mk` and `environment=production`; exclude
`synthetic_verification=true`/`verification_run_id` when constructing broader views.

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
input or content is serialized. Diagnostic consent is v4 and destination-bound: older consent does
not silently authorize this expanded schema.

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

## Synthetic end-to-end verification

After an explicitly approved project is selected, set its public ingest key and
US/EU host in the environment, set `ONE_MK_POSTHOG_ENVIRONMENT=verification`, then run:

```sh
cargo run --locked -p one-mk-ctl --example verify_posthog -- --confirm-synthetic
```

This creates a throwaway home/identity and uses the actual adapter for eight
synthetic events: app-open and settings-open, then a successful transfer outcome, failed transfer outcome and
safe exception, repeated after consent withdrawal/re-enable rotates identity.
All carry `synthetic_verification=true` and one fresh `verification_run_id` UUID.
The harness verifies old events cannot send after withdrawal or under renewed
consent. It prints UUIDs before attempting sends, checks transport acceptance,
and clears local consent afterward. It never reads the normal oneMK home.
Query all eight UUIDs and the grouped exception in the chosen project before
claiming end-to-end visibility. Unit tests do not run this live command.

On 2026-11-13, the approved shared EU project 347782 acknowledged all six events
for run `31dcfba6-1b42-4008-aacc-eaf084fadec6`. The expanded activity harness also
acknowledged eight events for run `6596d1c3-567b-421b-b048-b332e9c861b4`, including
`app_opened` and `settings_opened`. Provider-side visibility/grouping is checked
separately; acknowledgment alone is not proof. The initial run
`7150dfc2-b087-40b1-af2c-5728cc09740e` revealed that the current endpoint returns
`{"status":"Ok"}`: an event was ingested although the old local parser rejected
its acknowledgment. The parser now accepts exact `"Ok"` or numeric `1`, with
regression tests; no old event was blindly resent.

This verifies synthetic operation/error transport and reset behavior, not real
peer transfers, GUI journeys, panic upload, all crash types or symbolication.
Symbol/source uploads require a separate explicit decision. Production service
configuration and opt-in remain separate from this temporary verification run.

Local validation for the shared-project patch: 117 ctl library tests, 18 focused
analytics contracts, CLI analytics wiring tests, affected all-target Clippy,
notices check, and a real disposable-home CLI status/on/repeated-on/environment
switch/reset/regrant/off journey passed on macOS arm64. This does not replace
GUI/peer-transfer/device validation. CI is intentionally not re-triggered by
this patch's skip-CI commit while the account is at its Actions storage limit;
prior-head multi-platform CI results are not represented as this head's results.

### Explicit isolated panic/source-frame verification

`--confirm-synthetic-panic` runs a separate disposable child process, deliberately
aborts that child, recovers its existing panic marker and sends one tagged
exception using the real adapter. It never aborts the running desktop app or
reads its home. Child environment is cleared except fixed verification inputs;
no raw panic text is transmitted. Disable core dumps before this optional test:

```sh
ulimit -c 0
export ONE_MK_POSTHOG_ENVIRONMENT=verification
cargo run --locked -p one-mk-ctl --example verify_posthog -- --confirm-synthetic-panic
```

On macOS arm64, marker/event `4ab11d52-0178-422a-bb26-062c225bb980` was
acknowledged after confirmed child SIGABRT and exact-build marker recovery. Its
initial single owned-source frame used `platform=custom` but provider processing
rejected it for missing `lang`; its raw event persisted without a grouped issue.
The corrected frame includes truthful `lang=rust`. This is a safe panic location,
not a full native backtrace or symbol upload. Provider processing/grouping must
be checked separately. The earlier no-frame handled-error probe could not
validate this raw-frame schema. Reference: [manual exception capture schema](https://posthog.com/docs/error-tracking/installation/manual).

The corrected real panic probe `4029d04b-f614-4617-8c36-63f878863eb1` was
acknowledged with `platform=custom` and `lang=rust`. The initial missing-language
probe remains negative evidence, not a successful issue. During parallel local
tests the retry-pump contract encountered one transient `WouldBlock`; the test
now follows the worker's bounded retry behavior for that specific lock result,
without swallowing transport or other I/O errors.

Provider processing of the second panic candidate
`4029d04b-f614-4617-8c36-63f878863eb1` required a `function` field as well as
`platform` and `lang`. That candidate also persisted without a grouped issue.
The corrected source-only frame supplies the fixed explicit placeholder
`[function omitted]`, not an invented function name or raw panic text. These
negative probes demonstrate why HTTP acceptance is not successful issue parsing.

Final provider validation succeeded for the real panic event/run
`be01d54b-666e-40cd-8697-1104d5eb4175`: grouped issue
`01a1d566-ca02-7f63-8c4e-7414b7a3b068`, language `rust`, owned source
`crates/ctl/examples/verify_posthog.rs`, line 11 / column 9, and explicit
`[function omitted]` were preserved. The frame is `resolved=false`; this is
**not full symbolication**. The six-event operation/reset run also produced
exactly six provider rows across two isolated identities and grouped issue
`01a1d554-b59a-7603-af5f-c1143e8fd3a1`.

Final follow-up validation: all 117 ctl library tests and affected all-target
Clippy pass after the `function` correction. The corrected provider frame,
not the two rejected raw candidates, is the successful panic evidence.


### Pre-merge destination consent correction

Persisted v4 consent includes only a random installation ID and a BLAKE3 digest
of app ID, environment, canonical capture host and public project key. Startup
validates that digest before adopting the ID. A changed project key or region
therefore requires explicit new consent even after a process restart; repeated
opt-in for the unchanged destination preserves the identity. Legacy v3 and
malformed records fail closed, without automatic migration. Enabling requires
valid configuration; withdrawal still works without configuration. No raw key
is stored in consent. Local restart/configuration tests cover both key and
region changes, unchanged identity, explicit re-consent and old-format denial.
No additional live events were sent for this correction.

### Additional desktop readiness checks

The destination-consent change is covered by rendered desktop checkbox clicks
in isolated subprocesses: configured opt-in/withdrawal and unconfigured failure
without creating an identity. The complete macOS desktop test binary passed
390 tests, including frame interactions, settings persistence, pairing/transfer
UI states and isolated service-adapter failures; the CLI analytics wiring tests
passed both checks. Desktop all-target clippy passed with warnings denied.
These are headless UI/test-adapter checks, not real paired-device transfers or
a launched desktop/service end-to-end ingestion claim. Linux and Windows runtime
validation, native faults/OS kills, and full symbolication remain unverified.
