// SPDX-License-Identifier: BSD-3-Clause
//! Explicit little-endian wire (de)serialization for the fixed-layout packet
//! structs. This replaces the `bincode` 1.x serde encoding the upstream crate
//! used (unmaintained, RUSTSEC-2025-0141): integers are fixed-width
//! little-endian, fixed arrays and empty structs carry no length prefix, and
//! decoding ignores trailing bytes but fails on short input.
use anyhow::{Result, bail};

/// Cursor over a byte slice that fails (instead of panicking) when the input
/// is too short.
pub(crate) struct Reader<'a> {
    buf: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Self { buf }
    }

    pub(crate) fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let Some((head, rest)) = self.buf.split_first_chunk::<N>() else {
            bail!(
                "unexpected end of input: need {N} more bytes, have {}",
                self.buf.len()
            );
        };
        self.buf = rest;
        Ok(*head)
    }

    pub(crate) fn u32(&mut self) -> Result<u32> {
        self.array::<4>().map(u32::from_le_bytes)
    }

    pub(crate) fn u64(&mut self) -> Result<u64> {
        self.array::<8>().map(u64::from_le_bytes)
    }
}

/// A fixed-layout little-endian wire struct.
pub(crate) trait Wire: Sized {
    fn write_le(&self, out: &mut Vec<u8>);
    fn read_le(reader: &mut Reader<'_>) -> Result<Self>;

    /// Decode from the start of `buf`; bytes after the struct are ignored.
    fn from_le_bytes(buf: &[u8]) -> Result<Self> {
        Self::read_le(&mut Reader::new(buf))
    }
}
