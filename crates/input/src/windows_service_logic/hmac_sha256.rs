//! HMAC-SHA256 (RFC 2104) over `sha2`, with the three calls the handshake
//! needs. It replaces `ring::hmac` here so the helper's pure logic builds
//! for every target from any machine (`ring` needs a C toolchain for the
//! target, which a cross-check from Linux does not have); `sha2` is pure Rust
//! and is already part of the build through `one-mk-net`.
//!
//! Checked against the RFC 4231 test vectors below.

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

const BLOCK: usize = 64;

/// An HMAC-SHA256 key, already padded to the block size.
pub struct Key {
    ipad: [u8; BLOCK],
    opad: [u8; BLOCK],
}

impl Key {
    pub fn new(key: &[u8]) -> Self {
        let mut block = [0u8; BLOCK];
        if key.len() > BLOCK {
            let hashed = Sha256::digest(key);
            block[..hashed.len()].copy_from_slice(&hashed);
        } else {
            block[..key.len()].copy_from_slice(key);
        }
        let (mut ipad, mut opad) = ([0u8; BLOCK], [0u8; BLOCK]);
        for i in 0..BLOCK {
            ipad[i] = block[i] ^ 0x36;
            opad[i] = block[i] ^ 0x5c;
        }
        block.zeroize();
        Key { ipad, opad }
    }
}

impl Drop for Key {
    fn drop(&mut self) { self.ipad.zeroize(); self.opad.zeroize(); }
}

/// The 32-byte tag for `data`.
pub fn sign(key: &Key, data: &[u8]) -> [u8; 32] {
    let mut inner = Sha256::new();
    inner.update(key.ipad);
    inner.update(data);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(key.opad);
    outer.update(inner);
    let mut tag = [0u8; 32];
    tag.copy_from_slice(&outer.finalize());
    tag
}

/// Whether `tag` is the tag for `data`, compared without stopping at the
/// first difference.
pub fn verify(key: &Key, data: &[u8], tag: &[u8]) -> bool {
    let expected = sign(key, data);
    if tag.len() != expected.len() {
        return false;
    }
    bool::from(expected.ct_eq(tag))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn rfc_4231_test_vectors() {
        // Test case 1.
        assert_eq!(
            hex(&sign(&Key::new(&[0x0b; 20]), b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        // Test case 2.
        assert_eq!(
            hex(&sign(&Key::new(b"Jefe"), b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test case 3.
        assert_eq!(
            hex(&sign(&Key::new(&[0xaa; 20]), &[0xdd; 50])),
            "773ea91e36800e46854db8ebd09181a72959098b3ef8c122d9635514ced565fe"
        );
        // Test case 6: a key longer than the block size is hashed first.
        assert_eq!(
            hex(&sign(
                &Key::new(&[0xaa; 131]),
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn verify_accepts_only_the_right_tag() {
        let key = Key::new(&[7; 32]);
        let tag = sign(&key, b"message");
        assert!(verify(&key, b"message", &tag));
        assert!(!verify(&key, b"messagf", &tag));
        assert!(!verify(&Key::new(&[8; 32]), b"message", &tag));
        assert!(!verify(&key, b"message", &tag[..31]), "a short tag");
        let mut flipped = tag;
        flipped[31] ^= 1;
        assert!(!verify(&key, b"message", &flipped));
        assert!(!verify(&key, b"message", &[]), "an empty tag");
    }
}
