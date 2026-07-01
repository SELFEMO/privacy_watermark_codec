use std::{collections::HashMap, sync::OnceLock};

const CODEWORD_BITS: usize = 31;
const DATA_BITS: usize = 16;
const PARITY_BITS: usize = CODEWORD_BITS - DATA_BITS;
const GENERATOR: u32 = 0x8faf;
const ERROR_CORRECTION_LIMIT: usize = 3;

#[derive(Debug, Clone)]
pub struct DecodeOutcome {
    pub bytes: Vec<u8>,
    pub corrected_codewords: usize,
}

pub fn encode_bytes(input: &[u8]) -> Vec<bool> {
    let mut bits = Vec::with_capacity(encoded_bit_len(input.len()));
    for pair in input.chunks(2) {
        let high = pair[0] as u16;
        let low = pair.get(1).copied().unwrap_or_default() as u16;
        let data = (high << 8) | low;
        push_codeword_bits(encode_word(data), &mut bits);
    }
    bits
}

pub fn decode_bits(bits: &[bool], output_len: usize) -> Option<DecodeOutcome> {
    let required_bits = encoded_bit_len(output_len);
    if bits.len() < required_bits {
        return None;
    }

    let mut bytes = Vec::with_capacity(output_len);
    let mut corrected_codewords = 0_usize;
    for chunk in bits[..required_bits].chunks_exact(CODEWORD_BITS) {
        let received = chunk.iter().fold(0_u32, |value, bit| (value << 1) | u32::from(*bit));
        let syndrome = polynomial_mod(received);
        let (corrected, changed) = if syndrome == 0 {
            (received, false)
        } else {
            let mask = *syndrome_table().get(&syndrome)?;
            (received ^ mask, true)
        };
        if polynomial_mod(corrected) != 0 {
            return None;
        }
        corrected_codewords += usize::from(changed);
        let data = (corrected >> PARITY_BITS) as u16;
        bytes.push((data >> 8) as u8);
        if bytes.len() < output_len {
            bytes.push(data as u8);
        }
    }

    Some(DecodeOutcome {
        bytes,
        corrected_codewords,
    })
}

pub fn encoded_bit_len(byte_len: usize) -> usize {
    byte_len.div_ceil(2) * CODEWORD_BITS
}

fn encode_word(data: u16) -> u32 {
    let shifted = (data as u32) << PARITY_BITS;
    shifted | polynomial_mod(shifted)
}

fn push_codeword_bits(word: u32, bits: &mut Vec<bool>) {
    for shift in (0..CODEWORD_BITS).rev() {
        bits.push(((word >> shift) & 1) == 1);
    }
}

fn syndrome_table() -> &'static HashMap<u32, u32> {
    static TABLE: OnceLock<HashMap<u32, u32>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = HashMap::new();

        // BCH 的综合表只覆盖三位以内的错误模式，这样能把旧 Hamming 每码字一位纠错提升到三位纠错，同时避免引入外部依赖。
        // The BCH syndrome table covers only error patterns up to three bits, upgrading old Hamming one-bit correction to three-bit correction without adding dependencies.
        for weight in 1..=ERROR_CORRECTION_LIMIT {
            enumerate_error_masks(weight, 0, 0, &mut |mask| {
                table.entry(polynomial_mod(mask)).or_insert(mask);
            });
        }
        table
    })
}

fn enumerate_error_masks<F>(remaining: usize, start: usize, mask: u32, visit: &mut F)
where
    F: FnMut(u32),
{
    if remaining == 0 {
        visit(mask);
        return;
    }

    for bit in start..=CODEWORD_BITS - remaining {
        enumerate_error_masks(remaining - 1, bit + 1, mask | (1_u32 << bit), visit);
    }
}

fn polynomial_mod(mut value: u32) -> u32 {
    for shift in (PARITY_BITS..CODEWORD_BITS).rev() {
        if ((value >> shift) & 1) == 1 {
            value ^= GENERATOR << (shift - PARITY_BITS);
        }
    }
    value & ((1_u32 << PARITY_BITS) - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrects_three_bits_per_codeword() {
        let input = b"watermark-bch";
        let mut encoded = encode_bytes(input);
        for chunk in encoded.chunks_mut(CODEWORD_BITS) {
            chunk[0] = !chunk[0];
            chunk[7] = !chunk[7];
            chunk[18] = !chunk[18];
        }
        let decoded = decode_bits(&encoded, input.len()).unwrap();
        assert_eq!(decoded.bytes, input);
        assert_eq!(decoded.corrected_codewords, input.len().div_ceil(2));
    }
}
