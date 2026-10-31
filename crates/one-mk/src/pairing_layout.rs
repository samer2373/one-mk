use one_mk_net::{Device, Fingerprint, PlacedBy, TrustStore};
use one_mk_proto::{Side, pairing_layout::next_side, sanitize_name};
use std::net::SocketAddr;

pub const AUTOMATIC_VERSION_LIMIT: u64 = 16;

pub fn choose(trust: &TrustStore, fingerprint: Fingerprint, initiator: bool) -> Device {
    if let Some(existing) = trust.get(&fingerprint) {
        let mut device = existing.clone();
        if device.placed_by == PlacedBy::Default && device.learnable() { device.placed_by = PlacedBy::Pairing; }
        device.layout_version = device.layout_version.max(AUTOMATIC_VERSION_LIMIT + 1);
        return device;
    }
    let occupied: Vec<_> = Side::ALL.into_iter().filter(|side| {
        trust.devices().iter().any(|device| device.placed_by != PlacedBy::Pending && device.side() == *side)
    }).collect();
    let side = next_side(occupied.iter().copied());
    Device {
        fingerprint: fingerprint.to_string(),
        side: side.unwrap_or(Side::Left).to_string(),
        layout_version: if side.is_some() { 1 + occupied.len() as u64 * 2 + u64::from(initiator) } else { AUTOMATIC_VERSION_LIMIT },
        placed_by: if side.is_some() { PlacedBy::Pairing } else { PlacedBy::Pending },
        ..Device::default()
    }
}

pub fn warning(name: &str) -> String {
    format!("{}: placement needed; automatic edge placement is unavailable. Use `one-mk layout \"{}\" <left|right|top|bottom> --offset DX,DY`.", sanitize_name(name), sanitize_name(name))
}

pub fn remember(trust: &mut TrustStore, fingerprint: Fingerprint, name: &str, address: SocketAddr, initiator: bool) -> Device {
    let placement = choose(trust, fingerprint, initiator);
    let (version, origin) = (placement.layout_version, placement.placed_by);
    trust.upsert(Device { name: name.to_owned(), addrs: vec![address.to_string()], ..placement });
    let device = trust.get_mut(&fingerprint).expect("just paired");
    device.layout_version = version;
    device.placed_by = origin;
    device.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooses_free_sides_and_reserves_pending_without_moving_anything() {
        let home = tempfile::tempdir().unwrap();
        let mut trust = TrustStore::load(home.path()).unwrap();
        for (index, side) in [Side::Left, Side::Right, Side::Top, Side::Bottom].into_iter().enumerate() {
            let fingerprint = Fingerprint([index as u8; 32]);
            let device = remember(&mut trust, fingerprint, "peer", "127.0.0.1:12345".parse().unwrap(), false);
            assert_eq!(device.side(), side);
            assert!(device.learnable());
        }
        let device = choose(&trust, Fingerprint([9; 32]), true);
        assert_eq!(device.placed_by, PlacedBy::Pending);
        assert!(!device.learnable());
        assert_eq!(trust.devices().iter().map(Device::side).collect::<Vec<_>>(), [Side::Left, Side::Right, Side::Top, Side::Bottom]);
    }

    #[test]
    fn more_occupied_sides_then_initiator_win_the_existing_layout_sync() {
        let home = tempfile::tempdir().unwrap();
        let mut trust = TrustStore::load(home.path()).unwrap();
        let responder = choose(&trust, Fingerprint([1; 32]), false);
        let initiator = choose(&trust, Fingerprint([1; 32]), true);
        assert!(initiator.layout_version > responder.layout_version);
        trust.upsert(initiator.clone());
        let crowded = choose(&trust, Fingerprint([2; 32]), false);
        assert!(crowded.layout_version > initiator.layout_version);
    }

    #[test]
    fn repairing_preserves_offset_origin_and_settings_even_when_full() {
        let home = tempfile::tempdir().unwrap();
        let mut trust = TrustStore::load(home.path()).unwrap();
        let fingerprint = Fingerprint([1; 32]);
        trust.upsert(Device {
            fingerprint: fingerprint.to_string(), side: "bottom".into(), offset: Some((50, 1080)),
            layout_version: 123456, placed_by: PlacedBy::User, swap_ctrl_cmd: one_mk_net::Swap::On, os: 3,
            ..Device::default()
        });
        let device = remember(&mut trust, fingerprint, "renamed", "127.0.0.1:12345".parse().unwrap(), true);
        assert_eq!((device.side(), device.offset, device.layout_version), (Side::Bottom, Some((50, 1080)), 123456));
        assert_eq!((device.placed_by, device.swap_ctrl_cmd, device.os), (PlacedBy::User, one_mk_net::Swap::On, 3));
        assert_eq!(device.name, "renamed");
    }

    #[test]
    fn repairing_preserves_learned_confirmed_and_pending_origins() {
        let home = tempfile::tempdir().unwrap();
        for origin in [PlacedBy::Learned, PlacedBy::Confirmed, PlacedBy::Pending] {
            let mut trust = TrustStore::load(home.path()).unwrap();
            let fingerprint = Fingerprint([1; 32]);
            trust.upsert(Device { fingerprint: fingerprint.to_string(), side: "top".into(), placed_by: origin, ..Device::default() });
            let device = remember(&mut trust, fingerprint, "peer", "127.0.0.1:12345".parse().unwrap(), false);
            assert_eq!((device.side(), device.placed_by), (Side::Top, origin));
        }
    }
}
