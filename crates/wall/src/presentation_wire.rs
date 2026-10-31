//! Bounded presentation payload only; this is not an authenticated IPC service.
//! Credentials and live consent must be supplied independently to the Gate.

use crate::presentation::{PresentationPlan, Rect, Slice};

pub const MAX_PAYLOAD: usize = 4096;
pub const MAX_SLICES: usize = 16;
pub const MAX_SCREEN_BYTES: usize = 256;
const HEADER_BYTES: usize = 94;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Bounds, Version, Flags, Screen, Trailing }

fn valid_screen(screen: &str) -> bool {
    !screen.is_empty() && screen.len() <= MAX_SCREEN_BYTES && !screen.chars().any(char::is_control)
}

pub fn encode(plan: &PresentationPlan) -> Result<Vec<u8>, Error> {
    if plan.slices.len() > MAX_SLICES { return Err(Error::Bounds); }
    let mut length = HEADER_BYTES;
    for slice in &plan.slices {
        if !valid_screen(&slice.screen) { return Err(Error::Screen); }
        length += 18 + slice.screen.len();
    }
    if length > MAX_PAYLOAD { return Err(Error::Bounds); }
    let mut bytes = Vec::with_capacity(length);
    bytes.extend_from_slice(b"WPLN");
    bytes.push(1);
    bytes.push(u8::from(plan.looping) | (u8::from(plan.paused) << 1));
    bytes.extend_from_slice(&[0, 0]);
    bytes.extend_from_slice(&plan.generation);
    bytes.extend_from_slice(&plan.consent_revision.to_le_bytes());
    bytes.extend_from_slice(&plan.sequence.to_le_bytes());
    bytes.extend_from_slice(&plan.layout_serial.to_le_bytes());
    bytes.extend_from_slice(&plan.media_id);
    write_rect(&mut bytes, plan.canvas);
    bytes.extend_from_slice(&(plan.slices.len() as u16).to_le_bytes());
    for slice in &plan.slices {
        bytes.extend_from_slice(&(slice.screen.len() as u16).to_le_bytes());
        bytes.extend_from_slice(slice.screen.as_bytes());
        write_rect(&mut bytes, slice.canvas);
    }
    Ok(bytes)
}

fn write_rect(bytes: &mut Vec<u8>, rect: Rect) {
    bytes.extend_from_slice(&rect.x.to_le_bytes());
    bytes.extend_from_slice(&rect.y.to_le_bytes());
    bytes.extend_from_slice(&rect.width.to_le_bytes());
    bytes.extend_from_slice(&rect.height.to_le_bytes());
}

struct Reader<'a> { remaining: &'a [u8] }

impl Reader<'_> {
    fn word<const LENGTH: usize>(&mut self) -> Result<[u8; LENGTH], Error> {
        let (word, remaining) = self.remaining.split_at_checked(LENGTH).ok_or(Error::Bounds)?;
        self.remaining = remaining;
        word.try_into().map_err(|_| Error::Bounds)
    }

    fn rect(&mut self) -> Result<Rect, Error> {
        Ok(Rect { x: i32::from_le_bytes(self.word()?), y: i32::from_le_bytes(self.word()?),
            width: u32::from_le_bytes(self.word()?), height: u32::from_le_bytes(self.word()?) })
    }
}

pub fn decode(bytes: &[u8]) -> Result<PresentationPlan, Error> {
    if !(HEADER_BYTES..=MAX_PAYLOAD).contains(&bytes.len()) { return Err(Error::Bounds); }
    let mut reader = Reader { remaining: bytes };
    if reader.word::<4>()? != *b"WPLN" || reader.word::<1>()? != [1] {
        return Err(Error::Version);
    }
    let flags = reader.word::<1>()?[0];
    if flags & !3 != 0 || reader.word::<2>()? != [0, 0] { return Err(Error::Flags); }
    let generation = reader.word()?;
    let consent_revision = u64::from_le_bytes(reader.word()?);
    let sequence = u64::from_le_bytes(reader.word()?);
    let layout_serial = u32::from_le_bytes(reader.word()?);
    let media_id = reader.word()?;
    let canvas = reader.rect()?;
    let count = usize::from(u16::from_le_bytes(reader.word()?));
    if count > MAX_SLICES { return Err(Error::Bounds); }
    let mut slices = Vec::with_capacity(count);
    for _ in 0..count {
        let length = usize::from(u16::from_le_bytes(reader.word()?));
        if length == 0 || length > MAX_SCREEN_BYTES { return Err(Error::Screen); }
        let (screen, remaining) = reader.remaining.split_at_checked(length).ok_or(Error::Bounds)?;
        let screen = std::str::from_utf8(screen).map_err(|_| Error::Screen)?;
        if !valid_screen(screen) { return Err(Error::Screen); }
        reader.remaining = remaining;
        slices.push(Slice { screen: screen.to_owned(), canvas: reader.rect()? });
    }
    if !reader.remaining.is_empty() { return Err(Error::Trailing); }
    Ok(PresentationPlan { generation, consent_revision, sequence, layout_serial, media_id,
        canvas, slices, looping: flags & 1 != 0, paused: flags & 2 != 0 })
}
