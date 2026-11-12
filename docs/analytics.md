# Optional PostHog analytics

Analytics are **off by default**. No SDK autocapture, session replay, crash
reports, input, clipboard content, files, paths, peer names, addresses or machine
identifiers are collected. The desktop window itself is not instrumented.

To explicitly opt in for a daemon/CLI process, set all three variables in its
launch environment (do not commit a real key):

```sh
export ONE_MK_ANALYTICS_OPT_IN=true
export ONE_MK_POSTHOG_KEY=phc_YOUR_PUBLIC_PROJECT_INGEST_KEY
export ONE_MK_POSTHOG_HOST=https://eu.i.posthog.com
```

Choose your existing project's region: `https://eu.i.posthog.com` or
`https://us.i.posthog.com`. No other destinations or redirects are accepted.
This is the public project ingest key, never a personal API key. Nothing is
sent if any configuration is missing or invalid. Environment settings do not
automatically apply to an already-running daemon or OS login item.

Unset `ONE_MK_ANALYTICS_OPT_IN` and restart the process to opt out. There is no
analytics state or queue on disk. Each process gets a new random session ID,
unrelated to its device identity. Events in that process share it; there is
no cross-session retention or unique-user metric. No person profiles are
created. GeoIP enrichment is disabled; the ingestion service still receives
the network source IP as with any HTTPS request. Existing events can be
removed using the project's PostHog data-management controls.

Allowlisted events:

- `onemk_daemon_launch_requested`: daemon CLI invoked, not a readiness signal.
- `onemk_daemon_run_finished`: daemon invocation returned.
- `onemk_pairing_command_finished`: pair, join or invite invocation returned.
- `onemk_layout_command_finished`: layout invocation returned.
- `onemk_profile_command_finished`: profile invocation returned.

Events include only app version, random session ID, profile/GeoIP suppression
flags and (for finished commands) a success boolean. A command event may be a
read-only inspection or unsuccessful attempt, not necessarily a feature change.
Errors and command arguments are never included. Other commands are untracked.

Delivery is best effort, with no retries, persistence, response-body collection
or error logging. Each event gives its background sender at most 300 ms before
the app continues; short-lived processes may exit before delivery completes.
This deliberately favors app responsiveness over analytics completeness.
No real ingestion credentials or live events are needed for the unit tests:

```sh
cargo test --locked -p one-mk analytics::tests
cargo clippy --locked -p one-mk --all-targets -- -D warnings
```
