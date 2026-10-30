use crate::DecodeError;

pub const VERSION: u16 = 1;
pub const CONTROL_TAG: u8 = 25;
pub const BODY_LEN: usize = 112;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Version, Renderer, Canvas, ConsentRequest, ConsentGrant, ConsentRevoke }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    pub kind: Kind,
    pub coordinator: [u8;32],
    pub epoch: [u8;16],
    pub sequence: u64,
    pub descriptor: [u8;48],
}

impl Message {
    pub fn encode(self) -> [u8;BODY_LEN] {
        let mut bytes = [0;BODY_LEN];
        bytes[..2].copy_from_slice(&VERSION.to_le_bytes());
        bytes[2] = self.kind as u8;
        bytes[8..40].copy_from_slice(&self.coordinator);
        bytes[40..56].copy_from_slice(&self.epoch);
        bytes[56..64].copy_from_slice(&self.sequence.to_le_bytes());
        bytes[64..].copy_from_slice(&self.descriptor);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() != BODY_LEN || bytes[..2] != VERSION.to_le_bytes() || bytes[3..8] != [0;5] {
            return Err(DecodeError::Invalid("wallpaper version, size or reserved fields"));
        }
        let kind = match bytes[2] {
            0 => Kind::Version, 1 => Kind::Renderer, 2 => Kind::Canvas,
            3 => Kind::ConsentRequest, 4 => Kind::ConsentGrant, 5 => Kind::ConsentRevoke,
            _ => return Err(DecodeError::Invalid("wallpaper message kind")),
        };
        let message = Self { kind, coordinator: bytes[8..40].try_into().unwrap(), epoch: bytes[40..56].try_into().unwrap(),
            sequence: u64::from_le_bytes(bytes[56..64].try_into().unwrap()), descriptor: bytes[64..].try_into().unwrap() };
        if message.epoch == [0;16] || message.sequence == 0 { return Err(DecodeError::Invalid("wallpaper epoch or sequence")); }
        match kind {
            Kind::Canvas => {
                let width = u32::from_le_bytes(message.descriptor[..4].try_into().unwrap());
                let height = u32::from_le_bytes(message.descriptor[4..8].try_into().unwrap());
                let devices = message.descriptor[8];
                let outputs = message.descriptor[9];
                if !(1..=2_000_000).contains(&width) || !(1..=2_000_000).contains(&height)
                    || !(1..=8).contains(&devices) || !(devices..=16).contains(&outputs) || message.descriptor[10..16] != [0;6] {
                    return Err(DecodeError::Invalid("wallpaper canvas bounds"));
                }
            }
            Kind::Renderer if message.descriptor[0] <= 1 && message.descriptor[1..] == [0;47] => {}
            _ if message.descriptor != [0;48] => return Err(DecodeError::Invalid("wallpaper descriptor")),
            _ => {}
        }
        Ok(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_envelope_roundtrips_and_rejects_mutated_fields() {
        for kind in [Kind::Version,Kind::Renderer,Kind::Canvas,Kind::ConsentRequest,Kind::ConsentGrant,Kind::ConsentRevoke] {
            let mut message = Message { kind, coordinator: [7;32],epoch: [9;16],sequence: 1,descriptor: [0;48] };
            if kind == Kind::Canvas {
                message.descriptor[..4].copy_from_slice(&3300u32.to_le_bytes());
                message.descriptor[4..8].copy_from_slice(&1200u32.to_le_bytes());
                message.descriptor[8] = 2; message.descriptor[9] = 2;
            }
            let bytes = message.encode();
            assert_eq!(Message::decode(&bytes),Ok(message));
            assert!(Message::decode(&bytes[..111]).is_err());
            let mut oversized = bytes.to_vec(); oversized.push(0);
            assert!(Message::decode(&oversized).is_err());
            for index in [0,2,3,7] {
                let mut mutated = bytes; mutated[index] = 255;
                assert!(Message::decode(&mutated).is_err());
            }
        }
    }
}
