//! Messages for bulk data: clipboard snapshots and file transfer. These
//! always travel on their own streams, never on the control stream, so a
//! large copy can't delay input.

use crate::codec::{DecodeError, Reader, Writer, frame};

/// A clipboard representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ClipFormat {
    /// UTF-8 plain text.
    Text = 1,
    /// UTF-8 HTML fragment.
    Html = 2,
    /// PNG image.
    Png = 3,
}

impl ClipFormat {
    pub fn from_u8(b: u8) -> Option<Self> {
        match b {
            1 => Some(Self::Text),
            2 => Some(Self::Html),
            3 => Some(Self::Png),
            _ => None,
        }
    }
}

/// First frame of a clipboard stream: which representations follow, and
/// their sizes. The payloads follow back to back, in this order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipHeader {
    pub items: Vec<(ClipFormat, u32)>,
}

impl ClipHeader {
    pub fn total(&self) -> u64 {
        self.items.iter().map(|(_, n)| u64::from(*n)).sum()
    }

    pub fn encode_frame(&self, out: &mut Vec<u8>) {
        frame(out, |w| {
            let n = self.items.len().min(16);
            w.u8(n as u8);
            for (f, len) in &self.items[..n] {
                w.u8(*f as u8);
                w.u32(*len);
            }
        });
    }

    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader(b);
        let n = r.u8()?;
        let items = (0..n)
            .map(|_| {
                let tag = r.u8()?;
                let f = ClipFormat::from_u8(tag).ok_or(DecodeError::UnknownTag(tag))?;
                Ok((f, r.u32()?))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { items })
    }
}

/// Messages on a file stream. Per file:
///
/// ```text
/// sender   Offer ──────────────►
///          ◄────────────── Accept { offset }   (or Reject, then next Offer)
///          size - offset raw bytes ──►
///          End { hash } ────────►
///          ◄────────────── Saved | Failed
/// ```
///
/// The sender finishes the stream after the last file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileMsg {
    /// The next file. `path` is relative and `/`-separated; the receiver
    /// validates it. `mtime` (ns since the Unix epoch) and `size` identify
    /// the file's version for resuming.
    Offer { path: String, size: u64, mtime: u64 },
    /// Send the file from `offset`; the receiver already has the bytes
    /// before it from an interrupted transfer.
    Accept { offset: u64 },
    /// The receiver refuses this file.
    Reject { reason: String },
    /// All bytes are sent. `hash` is BLAKE3 over the whole file.
    End { hash: [u8; 32] },
    /// Verified and saved, as `name` relative to the download folder.
    Saved { name: String },
    /// Not saved (integrity check or disk failure).
    Failed { reason: String },
}

impl FileMsg {
    /// Longest relative path accepted.
    pub const MAX_PATH: usize = 4096;
    /// Upper bound for a frame body.
    pub const MAX_LEN: usize = 8 * 1024;

    pub fn encode_frame(&self, out: &mut Vec<u8>) {
        frame(out, |w| self.encode_body(w));
    }

    fn encode_body(&self, w: &mut Writer) {
        match self {
            FileMsg::Offer { path, size, mtime } => {
                w.u8(1);
                w.long_str(path, Self::MAX_PATH);
                w.u64(*size);
                w.u64(*mtime);
            }
            FileMsg::Accept { offset } => {
                w.u8(2);
                w.u64(*offset);
            }
            FileMsg::Reject { reason } => {
                w.u8(3);
                w.long_str(reason, 1024);
            }
            FileMsg::End { hash } => {
                w.u8(4);
                w.bytes(hash);
            }
            FileMsg::Saved { name } => {
                w.u8(5);
                w.long_str(name, Self::MAX_PATH);
            }
            FileMsg::Failed { reason } => {
                w.u8(6);
                w.long_str(reason, 1024);
            }
        }
    }

    pub fn decode(b: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader(b);
        let m = match r.u8()? {
            1 => FileMsg::Offer { path: r.long_str()?, size: r.u64()?, mtime: r.u64()? },
            2 => FileMsg::Accept { offset: r.u64()? },
            3 => FileMsg::Reject { reason: r.long_str()? },
            4 => FileMsg::End { hash: r.take()? },
            5 => FileMsg::Saved { name: r.long_str()? },
            6 => FileMsg::Failed { reason: r.long_str()? },
            t => return Err(DecodeError::UnknownTag(t)),
        };
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(buf: &[u8]) -> Vec<&[u8]> {
        let mut out = Vec::new();
        let mut rest = buf;
        while !rest.is_empty() {
            let len = u16::from_le_bytes([rest[0], rest[1]]) as usize;
            out.push(&rest[2..2 + len]);
            rest = &rest[2 + len..];
        }
        out
    }

    #[test]
    fn clip_header_round_trip() {
        for h in [
            ClipHeader::default(),
            ClipHeader { items: vec![(ClipFormat::Text, 5), (ClipFormat::Html, 70), (ClipFormat::Png, u32::MAX)] },
        ] {
            let mut buf = Vec::new();
            h.encode_frame(&mut buf);
            assert_eq!(ClipHeader::decode(frames(&buf)[0]).unwrap(), h);
        }
        assert_eq!(ClipHeader::decode(&[1, 9, 0, 0, 0, 0]), Err(DecodeError::UnknownTag(9)));
        assert_eq!(ClipHeader::decode(&[2, 1, 0, 0, 0, 0]), Err(DecodeError::Truncated));
    }

    #[test]
    fn file_msg_round_trip() {
        let msgs = [
            FileMsg::Offer { path: "photos/2024/ünïcode name.jpg".into(), size: 1 << 40, mtime: 1_700_000_000_123 },
            FileMsg::Accept { offset: 0 },
            FileMsg::Accept { offset: u64::MAX },
            FileMsg::Reject { reason: "path escapes the download folder".into() },
            FileMsg::End { hash: [7; 32] },
            FileMsg::Saved { name: "photos/a (1).jpg".into() },
            FileMsg::Failed { reason: "integrity check failed".into() },
        ];
        let mut buf = Vec::new();
        for m in &msgs {
            m.encode_frame(&mut buf);
        }
        let got: Vec<_> = frames(&buf).into_iter().map(|f| FileMsg::decode(f).unwrap()).collect();
        assert_eq!(got, msgs);
    }

    #[test]
    fn long_paths_are_truncated_on_a_char_boundary() {
        let path = "é".repeat(FileMsg::MAX_PATH); // 2 bytes each
        let mut buf = Vec::new();
        FileMsg::Offer { path, size: 1, mtime: 2 }.encode_frame(&mut buf);
        assert!(buf.len() - 2 <= FileMsg::MAX_LEN);
        let FileMsg::Offer { path, .. } = FileMsg::decode(&buf[2..]).unwrap() else { panic!() };
        assert_eq!(path.len(), FileMsg::MAX_PATH);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(FileMsg::decode(&[]), Err(DecodeError::Truncated));
        assert_eq!(FileMsg::decode(&[99]), Err(DecodeError::UnknownTag(99)));
        assert_eq!(FileMsg::decode(&[4, 1, 2]), Err(DecodeError::Truncated));
        assert_eq!(FileMsg::decode(&[1, 2, 0, 0xff, 0xfe, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), Err(DecodeError::BadString));
    }
}
