use wayland_client::{Connection, Dispatch, QueueHandle, protocol::wl_registry};
use crate::{Monitor, Rect};

pub const UNSUPPORTED: &str = "This Wayland compositor does not provide layer-shell wallpaper surfaces. GNOME needs a shell extension.";

pub fn capabilities(layer_shell: bool, outputs: usize) -> Result<(), &'static str> {
    if !layer_shell { Err(UNSUPPORTED) }
    else if outputs == 0 { Err("No Wayland outputs are connected for wallpaper playback.") }
    else { Ok(()) }
}

#[derive(Default)]
struct Probe { layer_shell: bool, outputs: usize }
impl Dispatch<wl_registry::WlRegistry, ()> for Probe {
    fn event(state: &mut Self, _: &wl_registry::WlRegistry, event: wl_registry::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { interface, .. } = event {
            if interface == "zwlr_layer_shell_v1" { state.layer_shell = true; }
            if interface == "wl_output" { state.outputs += 1; }
        }
    }
}

pub fn available() -> Result<(), &'static str> {
    let connection = Connection::connect_to_env().map_err(|_| "Cannot connect to the Wayland compositor for wallpaper playback.")?;
    let mut queue = connection.new_event_queue();
    connection.display().get_registry(&queue.handle(), ());
    let mut probe = Probe::default();
    queue.roundtrip(&mut probe).map_err(|_| "Cannot read the Wayland wallpaper protocols.")?;
    capabilities(probe.layer_shell, probe.outputs).map_err(|reason| {
        // GNOME has no layer-shell: say what works there, not the protocol.
        let gnome = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.to_ascii_lowercase().contains("gnome"));
        if reason == UNSUPPORTED && gnome { "Video wallpapers aren't available on GNOME Wayland yet. Pictures still work." } else { reason }
    })
}

pub fn match_monitor<'a>(monitors: &'a [Monitor], computer: &str, names: &[&str], local: Rect) -> Option<&'a Monitor> {
    let candidates: Vec<_> = monitors.iter().filter(|monitor| monitor.computer == computer).collect();
    let named: Vec<_> = candidates.iter().copied().filter(|monitor| !monitor.name.is_empty()
        && (names.contains(&monitor.name.as_str()) || names.contains(&monitor.name.rsplit(" · ").next().unwrap_or_default()))).collect();
    if named.len() == 1 { return named.first().copied(); }
    if !named.is_empty() { return None; }
    let positioned: Vec<_> = candidates.into_iter().filter(|monitor| monitor.local == Some(local)).collect();
    if positioned.len() == 1 { positioned.first().copied() } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detection_requires_protocol_and_output_not_desktop_name() {
        assert_eq!(capabilities(false, 2), Err(UNSUPPORTED));
        assert!(capabilities(true, 0).is_err());
        assert!(capabilities(true, 2).is_ok());
    }
    #[test]
    fn names_win_then_unique_local_geometry_never_arbitrary_output_order() {
        let rect = Rect { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };
        let monitor = Monitor { computer: "pc".into(), name: "DP-1".into(), rect, local: Some(rect), scale: 1.0, online: true };
        let monitors = vec![monitor.clone(), Monitor { computer: "peer".into(), ..monitor.clone() }];
        assert_eq!(match_monitor(&monitors, "pc", &["DP-1"], Rect::default()), Some(&monitors[0]));
        assert_eq!(match_monitor(&monitors, "pc", &["old-name"], rect), Some(&monitors[0]));
        assert!(match_monitor(&monitors, "pc", &["wrong"], Rect::default()).is_none());
        assert!(match_monitor(&[monitor.clone(), monitor], "pc", &["DP-1"], rect).is_none());
    }
}
