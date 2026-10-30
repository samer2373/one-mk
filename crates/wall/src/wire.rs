//! Experimental fixed-size envelope, never the existing control codec.
//! 64 bytes: WCLK, version=1, kind=1(request)/2(reply), two reserved zero
//! bytes, generation:u64, sequence:u64, then t1..t4:f64 seconds. All numeric
//! fields little endian; bytes 56..64 hold session:u64. Requests have
//! t2=t3=t4=0; replies have t4=0 until
//! the receiver stamps arrival. Decoder rejects trailing data and non-finites.
use crate::{Error, Probe, time};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Message {
    pub reply: bool,
    pub probe: Probe,
}

impl Message {
    pub fn encode(self) -> Result<[u8; 64], Error> {
        self.validate()?;
        let mut bytes = [0; 64];
        bytes[..4].copy_from_slice(b"WCLK");
        bytes[4] = 1;
        bytes[5] = if self.reply { 2 } else { 1 };
        bytes[8..16].copy_from_slice(&self.probe.generation.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.probe.sequence.to_le_bytes());
        bytes[56..64].copy_from_slice(&self.probe.session.to_le_bytes());
        for (slot, value) in bytes[24..56].as_chunks_mut::<8>().0.iter_mut().zip([self.probe.t1, self.probe.t2, self.probe.t3, self.probe.t4]) {
            slot.copy_from_slice(&value.to_le_bytes());
        }
        Ok(bytes)
    }

    fn validate(self) -> Result<(), Error> {
        let probe = self.probe;
        for value in [probe.t1, probe.t2, probe.t3, probe.t4] { time(value)?; }
        if probe.t4 != 0.0 || (!self.reply && (probe.t2 != 0.0 || probe.t3 != 0.0)) || (self.reply && probe.t3 < probe.t2) {
            return Err(Error::InvalidOrder);
        }
        Ok(())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != 64 || &bytes[..4] != b"WCLK" || bytes[4] != 1 || ![1, 2].contains(&bytes[5]) || bytes[6..8] != [0, 0] {
            return Err(Error::Bounds);
        }
        let word = |start: usize| -> Result<[u8; 8], Error> { bytes[start..start + 8].try_into().map_err(|_| Error::Bounds) };
        let message = Self { reply: bytes[5] == 2, probe: Probe {
            session: u64::from_le_bytes(word(56)?),
            generation: u64::from_le_bytes(word(8)?), sequence: u64::from_le_bytes(word(16)?),
            t1: f64::from_le_bytes(word(24)?), t2: f64::from_le_bytes(word(32)?),
            t3: f64::from_le_bytes(word(40)?), t4: f64::from_le_bytes(word(48)?),
        }};
        message.validate()?;
        Ok(message)
    }
}
