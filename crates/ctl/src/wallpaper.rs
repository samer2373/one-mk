use crate::status::Status;
use one_mk_wallpaper::{Canvas, Monitor, Rect};

pub fn canvas(status: &Status) -> Canvas {
    let mut canvas = Canvas::default();
    let mut add = |id: &str, name: &str, screens: &[one_mk_proto::Rect], offset: (i32, i32), online| {
        for (index, screen) in screens.iter().enumerate() {
            canvas.monitors.push(Monitor { computer: id.to_owned(), name: format!("{name} · {}", index + 1), online, scale: 1.0,
                local: Some(Rect { x: screen.x.into(), y: screen.y.into(), width: screen.w.into(), height: screen.h.into() }),
                rect: Rect { x: f64::from(screen.x) + f64::from(offset.0), y: f64::from(screen.y) + f64::from(offset.1),
                    width: f64::from(screen.w), height: f64::from(screen.h) } });
        }
    };
    add(&status.id, &status.name, &status.screens, (0, 0), true);
    let local = one_mk_proto::Desktop::new(status.screens.clone()).bounds();
    for peer in &status.peers {
        let peer_bounds = one_mk_proto::Desktop::new(peer.screens.clone()).bounds();
        let offset = status.offset_of(&peer.id).unwrap_or(match peer.side {
            one_mk_proto::Side::Left => (local.x - peer_bounds.w - peer_bounds.x, local.y - peer_bounds.y),
            one_mk_proto::Side::Right => (local.x + local.w - peer_bounds.x, local.y - peer_bounds.y),
            one_mk_proto::Side::Top => (local.x - peer_bounds.x, local.y - peer_bounds.h - peer_bounds.y),
            one_mk_proto::Side::Bottom => (local.x - peer_bounds.x, local.y + local.h - peer_bounds.y),
        });
        add(&peer.id, &peer.name, &peer.screens, offset, peer.online);
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uses_layout_offsets_without_reflowing_offline_computers() {
        let status = Status::decode("ok\nname\tMac\nid\tmac\nscreen\tme\t-1920\t0\t1920\t1080\nscreen\tme\t0\t0\t1920\t1080\npeer\tpc\tright\t2\t0\tauto\tPC\nscreen\tpc\t0\t0\t2560\t1440\noffset\tpc\t1920\t-360\n").unwrap();
        let canvas = canvas(&status);
        assert_eq!(canvas.monitors.len(), 3);
        let peer = &canvas.monitors[2];
        assert_eq!((peer.rect.x, peer.rect.y), (1920.0, -360.0));
        assert!(!peer.online);
        assert_eq!(peer.local.unwrap().x, 0.0);
        assert_eq!(canvas.bounds().unwrap().width, 6400.0);
    }
}
