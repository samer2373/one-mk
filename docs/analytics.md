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
milestones, crashes, raw exceptions and stacks. Successful apply means the
installer returned, not that the updated process subsequently started.

No keyboard/mouse activity, clipboard contents, filenames, paths, peer names,
fingerprints, IP addresses, SSIDs, window titles, screenshots, recordings or
logs are included. There is no autocapture, replay or person profile creation.
GeoIP enrichment is disabled; PostHog still receives the network source IP at
the transport layer. Previously ingested data must be deleted in PostHog.

## Delivery and withdrawal

Delivery runs off the UI/input/runtime thread, without retries or a disk queue.
Each process has a queue of at most 32 events and a single sender; full queues
drop events. Limit: 60 events/minute/process and at most one of the same event
per second. These are sampled counts, not an exact activity/audit log. Multiple
short-lived CLI processes each have their own budget.

Consent and the queued identity are rechecked before dispatch and again after
connecting, immediately before HTTP writing. Withdrawal invalidates pending
events even if consent is subsequently re-enabled. Already-writing/in-flight
HTTPS cannot be retracted or reliably cancelled across processes. A sender
already blocked in DNS/TLS is not forcibly killed. Socket operations have
one-second timeouts; OS DNS resolution may take longer without blocking the app.
CLI exit waits at most 300 ms; the process may exit before delivery completes.
No ingestion acknowledgment or live delivery guarantee is provided.

## Verification

Tests use temporary homes and synthetic credentials, with no real event traffic:

```sh
cargo test --locked -p one-mk-ctl analytics::tests
cargo test --locked -p one-mk analytics
cargo clippy --locked -p one-mk -p one-mk-app -p one-mk-ctl --all-targets -- -D warnings
python3 scripts/third-party-notices.py --check
```

Validate real UI layouts and end-to-end ingestion with an explicitly opted-in
test installation and test PostHog project before release. Nothing in this PR
creates a project, activates production, deploys or merges the change.
