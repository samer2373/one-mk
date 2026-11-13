//! The log filter: `RUST_LOG`-style `target=level,...` directives parsed with
//! tracing-subscriber's `Targets`, which needs no regex engine (the `env-filter`
//! feature pulls in regex-automata and regex-syntax with their Unicode tables).
//!
//! Accepted syntax, the same as before for everything oneMK documents or uses:
//! a comma-separated list of `level` (the default for every target), `target`
//! (that target at trace) and `target=level` (prefix match on the module path),
//! levels `error|warn|info|debug|trace|off` in any case or `0`-`5`. Empty
//! segments are skipped and an empty value means `error`, as with `EnvFilter`.
//! An invalid value falls back to the verbosity default, as before. Span and
//! field directives (`[span{field=value}]=level`) are not supported; nothing
//! in oneMK uses them, so they count as invalid.

use tracing_subscriber::filter::{LevelFilter, Targets};

/// The filter for the given `RUST_LOG` value (`None` when unset), or `fallback`
/// (a directive string for the verbosity level) when it is unset or invalid.
pub fn parse(env: Option<&str>, fallback: &str) -> Targets {
    env.and_then(directives).unwrap_or_else(|| {
        directives(fallback).unwrap_or_else(|| Targets::new().with_default(LevelFilter::WARN))
    })
}

/// `Targets` from a directive list, or `None` when a directive is invalid.
fn directives(list: &str) -> Option<Targets> {
    let kept: Vec<&str> = list.split(',').filter(|s| !s.is_empty()).collect();
    if kept.is_empty() {
        return Some(Targets::new().with_default(LevelFilter::ERROR));
    }
    // A directive with a span or field part is not a plain target; refuse it
    // rather than quietly turning it into a different filter.
    if kept.iter().any(|d| d.contains('[') || d.contains('{')) {
        return None;
    }
    kept.join(",").parse().ok()
}

/// The filter for this process: `RUST_LOG` if it is set and valid, else `fallback`.
pub fn from_env(fallback: &str) -> Targets {
    parse(std::env::var("RUST_LOG").ok().as_deref(), fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::Level;
    use tracing_subscriber::{Registry, layer::SubscriberExt};

    // Whether an event at `level` from `target` passes the filter.
    fn on(t: &Targets, target: &str, level: Level) -> bool {
        // `Targets` has no direct query in every version; drive it as a layer filter
        // would, by asking its directive set through `would_enable`.
        t.would_enable(target, &level)
    }

    const FALLBACK: &str = "warn,mdns_sd=error,zbus=error,one_mk::live_wallpaper=info";
    const VERBOSE: &str =
        "debug,mdns_sd=warn,zbus=info,quinn=info,quinn_proto=info,rustls=info,arboard=warn";

    #[test]
    fn unset_or_invalid_uses_the_fallback() {
        for env in [None, Some("one_mk=loud"), Some("a=b=c"), Some("[span]=info")] {
            let t = parse(env, FALLBACK);
            assert!(on(&t, "one_mk::app", Level::WARN), "{env:?}");
            assert!(!on(&t, "one_mk::app", Level::INFO), "{env:?}");
            assert!(on(&t, "one_mk::live_wallpaper", Level::INFO), "{env:?}");
            assert!(!on(&t, "mdns_sd", Level::WARN), "{env:?}");
        }
    }

    #[test]
    fn default_level_plus_target_overrides() {
        // The value qa_resilience_e2e.rs gives the daemon.
        let t = parse(Some("warn,one_mk=info"), FALLBACK);
        assert!(on(&t, "one_mk::app", Level::INFO));
        assert!(on(&t, "one_mk", Level::INFO));
        assert!(!on(&t, "one_mk::app", Level::DEBUG));
        assert!(on(&t, "quinn", Level::WARN));
        assert!(!on(&t, "quinn", Level::INFO));
        // The fallback is not merged in.
        assert!(on(&t, "mdns_sd", Level::WARN));
    }

    #[test]
    fn levels_in_any_case_numeric_and_off() {
        assert!(on(&parse(Some("DEBUG"), FALLBACK), "x", Level::DEBUG));
        assert!(!on(&parse(Some("DEBUG"), FALLBACK), "x", Level::TRACE));
        assert!(on(&parse(Some("Trace"), FALLBACK), "x", Level::TRACE));
        assert!(on(&parse(Some("4"), FALLBACK), "x", Level::DEBUG));
        assert!(!on(&parse(Some("off"), FALLBACK), "x", Level::ERROR));
        let t = parse(Some("info,zbus=OFF,quinn=Warn"), FALLBACK);
        assert!(!on(&t, "zbus::connection", Level::ERROR));
        assert!(on(&t, "quinn", Level::WARN));
        assert!(!on(&t, "quinn", Level::INFO));
        assert!(on(&t, "other", Level::INFO));
    }

    #[test]
    fn bare_target_means_trace_and_without_a_default_others_are_off() {
        let t = parse(Some("one_mk::net"), FALLBACK);
        assert!(on(&t, "one_mk::net", Level::TRACE));
        assert!(!on(&t, "quinn", Level::ERROR));
        let t = parse(Some("quinn=debug"), FALLBACK);
        assert!(on(&t, "quinn", Level::DEBUG));
        assert!(!on(&t, "one_mk", Level::ERROR));
    }

    #[test]
    fn the_more_specific_target_wins_regardless_of_order() {
        for v in ["debug,one_mk=warn,one_mk::net=trace", "one_mk::net=trace,one_mk=warn,debug"] {
            let t = parse(Some(v), FALLBACK);
            assert!(on(&t, "one_mk::net::link", Level::TRACE), "{v}");
            assert!(!on(&t, "one_mk::app", Level::INFO), "{v}");
            assert!(on(&t, "other", Level::DEBUG), "{v}");
        }
    }

    #[test]
    fn empty_segments_are_skipped_and_empty_means_error() {
        let t = parse(Some("info,,one_mk=debug,"), FALLBACK);
        assert!(on(&t, "one_mk", Level::DEBUG));
        assert!(on(&t, "x", Level::INFO));
        assert!(!on(&t, "x", Level::DEBUG));
        let t = parse(Some(""), FALLBACK);
        assert!(on(&t, "x", Level::ERROR));
        assert!(!on(&t, "x", Level::WARN));
    }

    #[test]
    fn the_verbosity_defaults_parse_and_behave() {
        for d in [FALLBACK, VERBOSE, "trace"] {
            assert!(directives(d).is_some(), "{d}");
        }
        let v1 = parse(None, VERBOSE);
        assert!(on(&v1, "one_mk::app", Level::DEBUG));
        assert!(!on(&v1, "quinn_proto::connection", Level::DEBUG));
        assert!(on(&v1, "quinn_proto::connection", Level::INFO));
        assert!(!on(&v1, "arboard", Level::INFO));
    }

    #[test]
    fn it_works_as_a_per_layer_filter_like_the_daemon_installs_it() {
        use tracing_subscriber::Layer;
        let layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::sink)
            .with_filter(parse(Some("warn,one_mk=info"), FALLBACK));
        let subscriber = Registry::default().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "one_mk::x", "kept");
            tracing::debug!(target: "one_mk::x", "dropped");
        });
    }
}
