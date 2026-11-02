//! Production release verification key; the private key is held only by the owner and CI.
pub const PRODUCTION_KEY: bool = true;
pub const ADDITIONAL_PUBLIC_KEYS: &[[u8; 32]] = &[];
pub const PUBLIC_KEY: [u8; 32] = [
    0x1c, 0x96, 0xa6, 0x0f, 0x74, 0x11, 0xb8, 0x90,
    0x48, 0xa7, 0xc8, 0xe7, 0x33, 0xf1, 0xab, 0xcf,
    0x3b, 0x21, 0xd4, 0x59, 0xf5, 0x07, 0x09, 0x1c,
    0xce, 0xf7, 0x53, 0xf3, 0xfc, 0x88, 0xbf, 0xeb,
];

/// The public RFC 8032 test key, whose private seed is published in this
/// repository's tests: it must never be the production key.
const RFC8032_TEST: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7,
    0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25,
    0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];

const fn same(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut i = 0;
    while i < 32 {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

// Turning production checks on while the test key is still embedded would let
// anyone with the (public) test seed forge an update: refuse to compile.
const _: () = assert!(!PRODUCTION_KEY || !same(&PUBLIC_KEY, &RFC8032_TEST), "PRODUCTION_KEY is on but PUBLIC_KEY is still the RFC 8032 test key");
