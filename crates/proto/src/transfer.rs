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
///
/// A header with `pull` set and no items asks the peer to answer on the
/// same stream with its own clipboard (a header and payloads like this
/// one, without `pull`). The flag is a trailing byte that only a pull
/// carries, so an ordinary snapshot is byte-for-byte what it always was,
/// and an older peer reads a pull as an empty snapshot and ignores it (the
/// stream then ends with no answer, which the asker reports).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClipHeader {
    pub items: Vec<(ClipFormat, u32)>,
    pub pull: bool,
}

impl ClipHeader {
    /// Bit 0 of the optional trailing flags byte.
    const PULL: u8 = 1;

    /// Asks the peer for its clipboard.
    pub fn pull_request() -> Self {
        Self { items: Vec::new(), pull: true }
    }

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
            if self.pull {
                w.u8(Self::PULL);
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
        // Optional flags; bits this version doesn't know are ignored.
        let pull = !r.0.is_empty() && r.u8()? & Self::PULL != 0;
        Ok(Self { items, pull })
    }
}

/// The one byte a receiver may send back on a pushed clipboard stream, to
/// say what became of it. Senders that don't wait for it (the automatic
/// push) never read it, and receivers from before it existed never send it,
/// so it can be absent: no answer means "unknown", not "refused".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ClipAck {
    /// Now on the receiver's clipboard.
    Stored = 0,
    /// Every format in it is switched off at the receiver.
    FormatsOff = 1,
    /// Over the receiver's size limit.
    TooLarge = 2,
}

impl ClipAck {
    pub fn from_u8(b: u8) -> Option<Self> {
        match b {
            0 => Some(Self::Stored),
            1 => Some(Self::FormatsOff),
            2 => Some(Self::TooLarge),
            _ => None,
        }
    }
}

/// Which send a file belongs to, so the receiver can show one progress
/// line for a whole folder and one notification when it is done. All of
/// a sender's streams repeat the same value in every offer.
///
/// It rides as extra bytes after an `Offer`'s fixed fields, which an older
/// receiver never reads, and an offer without them (`id` 0) is what an
/// older sender sends: then the receiver only knows the files as they
/// are offered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Batch {
    /// Random per send; 0 means unknown.
    pub id: u64,
    /// Files in the whole send.
    pub files: u32,
    /// Bytes in the whole send.
    pub bytes: u64,
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
    Offer { path: String, size: u64, mtime: u64, batch: Batch },
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
            FileMsg::Offer { path, size, mtime, batch } => {
                w.u8(1);
                w.long_str(path, Self::MAX_PATH);
                w.u64(*size);
                w.u64(*mtime);
                if batch.id != 0 {
                    w.u64(batch.id);
                    w.u32(batch.files);
                    w.u64(batch.bytes);
                }
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
            1 => {
                let (path, size, mtime) = (r.long_str()?, r.u64()?, r.u64()?);
                let batch = if r.0.is_empty() {
                    Batch::default()
                } else {
                    Batch { id: r.u64()?, files: r.u32()?, bytes: r.u64()? }
                };
                FileMsg::Offer { path, size, mtime, batch }
            }
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
            ClipHeader { items: vec![(ClipFormat::Text, 5), (ClipFormat::Html, 70), (ClipFormat::Png, u32::MAX)], pull: false },
            ClipHeader::pull_request(),
        ] {
            let mut buf = Vec::new();
            h.encode_frame(&mut buf);
            assert_eq!(ClipHeader::decode(frames(&buf)[0]).unwrap(), h);
        }
        assert_eq!(ClipHeader::decode(&[1, 9, 0, 0, 0, 0]), Err(DecodeError::UnknownTag(9)));
        assert_eq!(ClipHeader::decode(&[2, 1, 0, 0, 0, 0]), Err(DecodeError::Truncated));
    }

    /// A snapshot is encoded exactly as before pull existed, and a pull is
    /// an empty snapshot plus one byte, so an old peer reads it as empty.
    #[test]
    fn clip_pull_is_additive() {
        let snapshot = ClipHeader { items: vec![(ClipFormat::Text, 5)], pull: false };
        let mut buf = Vec::new();
        snapshot.encode_frame(&mut buf);
        assert_eq!(buf, [6, 0, 1, 1, 5, 0, 0, 0]);

        let mut buf = Vec::new();
        ClipHeader::pull_request().encode_frame(&mut buf);
        let body = &buf[2..];
        assert_eq!(body, [0, 1]);
        // What an old decoder (it stops after the items) makes of it: an empty snapshot.
        assert_eq!(ClipHeader::decode(&body[..1]).unwrap(), ClipHeader::default());

        // Unknown flag bits are ignored; the pull bit alone decides.
        assert!(!ClipHeader::decode(&[0, 0xfe]).unwrap().pull);
        assert!(ClipHeader::decode(&[0, 0xff]).unwrap().pull);
        assert!(ClipHeader::decode(&[1, 1, 3, 0, 0, 0, 1]).unwrap().pull);
    }

    #[test]
    fn clip_ack_round_trip() {
        for a in [ClipAck::Stored, ClipAck::FormatsOff, ClipAck::TooLarge] {
            assert_eq!(ClipAck::from_u8(a as u8), Some(a));
        }
        assert_eq!(ClipAck::from_u8(3), None, "an unknown answer is not mistaken for a known one");
        assert_eq!(ClipAck::from_u8(255), None);
    }

    #[test]
    fn offer_batch_is_additive() {
        let batch = Batch { id: 0xdead_beef_0123, files: 40, bytes: 1 << 33 };
        let with = FileMsg::Offer { path: "a/b".into(), size: 7, mtime: 9, batch };
        let without = FileMsg::Offer { path: "a/b".into(), size: 7, mtime: 9, batch: Batch::default() };
        let (mut a, mut b) = (Vec::new(), Vec::new());
        with.encode_frame(&mut a);
        without.encode_frame(&mut b);
        assert_eq!(FileMsg::decode(&a[2..]).unwrap(), with);
        assert_eq!(FileMsg::decode(&b[2..]).unwrap(), without);
        // The old fixed fields come first and are unchanged; the batch is a tail.
        assert_eq!(a.len() - b.len(), 8 + 4 + 8);
        assert_eq!(&a[2..2 + b.len() - 2], &b[2..]);
        // A tail cut short is malformed, not silently "unknown batch".
        for cut in 1..20 {
            assert_eq!(FileMsg::decode(&a[2..a.len() - cut]), Err(DecodeError::Truncated), "cut {cut}");
        }
    }

    #[test]
    fn file_msg_round_trip() {
        let msgs = [
            FileMsg::Offer { path: "photos/2024/ünïcode name.jpg".into(), size: 1 << 40, mtime: 1_700_000_000_123, batch: Batch::default() },
            FileMsg::Offer { path: "x".into(), size: 1, mtime: 2, batch: Batch { id: 5, files: 3, bytes: 99 } },
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
        FileMsg::Offer { path, size: 1, mtime: 2, batch: Batch::default() }.encode_frame(&mut buf);
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
