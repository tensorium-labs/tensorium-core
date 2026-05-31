use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Hash256(pub [u8; 32]);

impl Hash256 {
    pub const ZERO: Self = Self([0u8; 32]);

    pub fn double_sha256(bytes: &[u8]) -> Self {
        let first = Sha256::digest(bytes);
        let second = Sha256::digest(first);
        let mut out = [0u8; 32];
        out.copy_from_slice(&second);
        Self(out)
    }

    pub fn leading_zero_bits(&self) -> u32 {
        let mut bits = 0u32;
        for byte in self.0 {
            if byte == 0 {
                bits += 8;
            } else {
                bits += byte.leading_zeros();
                break;
            }
        }
        bits
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }
}

impl Default for Hash256 {
    fn default() -> Self {
        Self::ZERO
    }
}

impl core::fmt::Debug for Hash256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl core::fmt::Display for Hash256 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_leading_zero_bits() {
        let mut bytes = [0xff; 32];
        bytes[0] = 0;
        bytes[1] = 0b0001_1111;
        assert_eq!(Hash256(bytes).leading_zero_bits(), 11);
    }
}
