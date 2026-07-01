#[derive(Debug, Clone)]
pub struct DecodeOutcome {
    pub bytes: Vec<u8>,
    pub corrected_codewords: usize,
}

pub fn encode_bytes(input: &[u8]) -> Vec<bool> {
    let mut bits = Vec::with_capacity(input.len() * 14);
    for &byte in input {
        bits.extend(encode_nibble(byte >> 4));
        bits.extend(encode_nibble(byte & 0x0f));
    }
    bits
}

pub fn decode_bits(bits: &[bool], output_len: usize) -> Option<DecodeOutcome> {
    if bits.len() < output_len * 14 {
        return None;
    }

    let mut nibbles = Vec::with_capacity(output_len * 2);
    let mut corrected_codewords = 0;
    for chunk in bits[..output_len * 14].chunks_exact(7) {
        let (nibble, corrected) = decode_codeword(chunk.try_into().ok()?);
        nibbles.push(nibble);
        corrected_codewords += usize::from(corrected);
    }

    let mut bytes = Vec::with_capacity(output_len);
    for pair in nibbles.chunks_exact(2) {
        bytes.push((pair[0] << 4) | pair[1]);
    }

    Some(DecodeOutcome {
        bytes,
        corrected_codewords,
    })
}

fn encode_nibble(nibble: u8) -> [bool; 7] {
    let d1 = (nibble & 0b1000) != 0;
    let d2 = (nibble & 0b0100) != 0;
    let d3 = (nibble & 0b0010) != 0;
    let d4 = (nibble & 0b0001) != 0;
    let p1 = d1 ^ d2 ^ d4;
    let p2 = d1 ^ d3 ^ d4;
    let p4 = d2 ^ d3 ^ d4;
    [p1, p2, d1, p4, d2, d3, d4]
}

fn decode_codeword(mut word: [bool; 7]) -> (u8, bool) {
    let s1 = word[0] ^ word[2] ^ word[4] ^ word[6];
    let s2 = word[1] ^ word[2] ^ word[5] ^ word[6];
    let s4 = word[3] ^ word[4] ^ word[5] ^ word[6];
    let syndrome = usize::from(s1) | (usize::from(s2) << 1) | (usize::from(s4) << 2);
    let corrected = syndrome != 0;
    if (1..=7).contains(&syndrome) {
        word[syndrome - 1] = !word[syndrome - 1];
    }

    let nibble = (u8::from(word[2]) << 3)
        | (u8::from(word[4]) << 2)
        | (u8::from(word[5]) << 1)
        | u8::from(word[6]);
    (nibble, corrected)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrects_one_bit_per_codeword() {
        let input = b"watermark";
        let mut encoded = encode_bytes(input);
        for chunk in encoded.chunks_mut(7) {
            chunk[2] = !chunk[2];
        }
        let decoded = decode_bits(&encoded, input.len()).unwrap();
        assert_eq!(decoded.bytes, input);
        assert_eq!(decoded.corrected_codewords, input.len() * 2);
    }
}
