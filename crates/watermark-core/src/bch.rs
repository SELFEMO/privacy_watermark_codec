use std::{collections::HashMap, sync::OnceLock};

const CODEWORD_BITS: usize = 31;
const DATA_BITS: usize = 16;
const PARITY_BITS: usize = CODEWORD_BITS - DATA_BITS;
const GENERATOR: u32 = 0x8faf;
const ERROR_CORRECTION_LIMIT: usize = 3;
const ERASURE_LIMIT: usize = 6;

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
        push_codeword_bits(encode_word((high << 8) | low), &mut bits);
    }
    bits
}

pub fn decode_bits(bits: &[bool], output_len: usize) -> Option<DecodeOutcome> {
    decode_bits_with_erasures(bits, &vec![false; bits.len()], output_len)
}

pub fn decode_bits_with_erasures(
    bits: &[bool],
    erasures: &[bool],
    output_len: usize,
) -> Option<DecodeOutcome> {
    let required_bits = encoded_bit_len(output_len);
    if bits.len() < required_bits || erasures.len() < required_bits {
        return None;
    }

    let mut bytes = Vec::with_capacity(output_len);
    let mut corrected_codewords = 0_usize;
    for (bit_chunk, erasure_chunk) in bits[..required_bits]
        .chunks_exact(CODEWORD_BITS)
        .zip(erasures[..required_bits].chunks_exact(CODEWORD_BITS))
    {
        // 按码字检查停止状态，使大型正文纠错也服从同一次请求的截止时间和取消信号。
        // Check each codeword so correction of a large payload obeys the same request deadline and cancellation signal.
        if crate::recovery::stopped() {
            return None;
        }
        let (word, changed) = decode_codeword(bit_chunk, erasure_chunk)?;
        corrected_codewords += usize::from(changed);
        let data = (word >> PARITY_BITS) as u16;
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
    ((byte_len + 1) / 2) * CODEWORD_BITS
}

pub(crate) fn erasure_candidates(
    bits: &[bool],
    observed: &[bool],
    reliability: &[f32],
    output_len: usize,
) -> Vec<DecodeOutcome> {
    static CODEBOOK: OnceLock<Vec<u32>> = OnceLock::new();
    let count = encoded_bit_len(output_len);
    if bits.len() < count || observed.len() < count || reliability.len() < count || output_len > 512
    {
        return Vec::new();
    }
    let missing = observed[..count].iter().filter(|value| !**value).count();
    if missing == 0 || missing * 10 > count * 4 {
        return Vec::new();
    }
    let book = CODEBOOK.get_or_init(|| (0..=u16::MAX).map(encode_word).collect());
    let mut paths = vec![(Vec::new(), 0.0_f32)];
    // 超过唯一擦除纠正界限时按软置信度保留有限候选，最终必须通过载荷 CRC 和 AEAD，不能把歧义列表当成已解出的正文。
    // Beyond unique erasure correction, retain a bounded confidence-ranked list; payload CRC and AEAD must resolve it before any text is accepted.
    for (word_index, (word_bits, word_observed)) in bits[..count]
        .chunks_exact(31)
        .zip(observed[..count].chunks_exact(31))
        .enumerate()
    {
        if crate::recovery::stopped() {
            return Vec::new();
        }
        let received = word_bits
            .iter()
            .fold(0_u32, |word, bit| (word << 1) | u32::from(*bit));
        let mask = word_observed
            .iter()
            .fold(0_u32, |word, bit| (word << 1) | u32::from(*bit));
        let erasures: Vec<_> = word_observed.iter().map(|value| !value).collect();
        let mut choices = Vec::new();
        if let Some((word, _)) = decode_codeword(word_bits, &erasures) {
            choices.push(((word >> 15) as u16, 0.0));
        } else {
            if mask.count_ones() < 16 {
                return Vec::new();
            }
            for (data, &word) in book.iter().enumerate() {
                let mut changed = (word ^ received) & mask;
                if changed.count_ones() > 3 {
                    continue;
                }
                let mut cost = 0.0;
                while changed != 0 {
                    let position = changed.trailing_zeros() as usize;
                    cost += reliability[word_index * 31 + 30 - position].max(0.02);
                    changed &= changed - 1;
                }
                choices.push((data as u16, cost));
            }
            choices.sort_by(|a, b| a.1.total_cmp(&b.1));
            choices.truncate(4);
        }
        if choices.is_empty() {
            return Vec::new();
        }
        let mut next = Vec::new();
        for (path, cost) in &paths {
            for (word, extra) in &choices {
                let mut bytes = path.clone();
                bytes.extend_from_slice(&word.to_be_bytes());
                next.push((bytes, cost + extra));
            }
        }
        // 有界束搜索限制最坏情况下的时间和内存，认证失败时返回失败而不是无限枚举。
        // A bounded beam limits worst-case time and memory, returning failure instead of enumerating indefinitely when authentication cannot be achieved.
        next.sort_by(|a, b| a.1.total_cmp(&b.1));
        next.truncate(256);
        paths = next;
    }
    let corrected = observed[..count]
        .chunks(31)
        .zip(bits[..count].chunks(31))
        .filter(|(seen, values)| {
            seen.iter().any(|value| !*value)
                || polynomial_mod(
                    values
                        .iter()
                        .fold(0_u32, |word, value| (word << 1) | u32::from(*value)),
                ) != 0
        })
        .count();
    paths
        .into_iter()
        .map(|(mut bytes, _)| {
            bytes.truncate(output_len);
            DecodeOutcome {
                bytes,
                corrected_codewords: corrected,
            }
        })
        .collect()
}

fn decode_codeword(bits: &[bool], erasures: &[bool]) -> Option<(u32, bool)> {
    let received = bits
        .iter()
        .fold(0_u32, |value, bit| (value << 1) | u32::from(*bit));
    let positions: Vec<usize> = erasures
        .iter()
        .enumerate()
        .filter_map(|(index, erased)| erased.then_some(CODEWORD_BITS - 1 - index))
        .collect();
    if positions.len() > ERASURE_LIMIT {
        return None;
    }

    let assignments = 1_usize << positions.len();
    let mut best: Option<(u32, usize)> = None;
    for assignment in 0..assignments {
        let mut candidate = received;
        for (index, position) in positions.iter().enumerate() {
            let mask = 1_u32 << position;
            if ((assignment >> index) & 1) == 1 {
                candidate |= mask;
            } else {
                candidate &= !mask;
            }
        }
        let syndrome = polynomial_mod(candidate);
        let corrected = if syndrome == 0 {
            candidate
        } else if let Some(mask) = syndrome_table().get(&syndrome) {
            candidate ^ *mask
        } else {
            continue;
        };
        if polynomial_mod(corrected) != 0 {
            continue;
        }
        let observed_mask = positions
            .iter()
            .fold((1_u32 << CODEWORD_BITS) - 1, |mask, position| {
                mask & !(1_u32 << position)
            });
        let observed_changes = ((corrected ^ received) & observed_mask).count_ones() as usize;
        if observed_changes > ERROR_CORRECTION_LIMIT
            || observed_changes * 2 + positions.len() > ERROR_CORRECTION_LIMIT * 2
        {
            continue;
        }
        if best
            .map(|(_, score)| observed_changes < score)
            .unwrap_or(true)
        {
            best = Some((corrected, observed_changes));
        }
    }

    // 对裁剪造成的“完全未观测位”枚举有限擦除值，并遵守 2×错误数+擦除数≤6 的唯一解码界限，避免把缺失块误判成确定的 0 或选中歧义码字。
    // Enumerating bounded values for completely unobserved crop erasures while enforcing 2×errors+erasures≤6 avoids treating missing blocks as definite zeros or selecting an ambiguous codeword.
    best.map(|(word, score)| (word, !positions.is_empty() || score > 0))
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
        assert_eq!(decode_bits(&encoded, input.len()).unwrap().bytes, input);
    }

    #[test]
    fn recovers_six_erasures_per_codeword() {
        let input = b"crop-erasure";
        let encoded = encode_bytes(input);
        let mut damaged = encoded.clone();
        let mut erasures = vec![false; encoded.len()];
        for (bit_chunk, erased_chunk) in damaged
            .chunks_mut(CODEWORD_BITS)
            .zip(erasures.chunks_mut(CODEWORD_BITS))
        {
            for index in [1, 4, 9, 14, 20, 27] {
                bit_chunk[index] = false;
                erased_chunk[index] = true;
            }
        }
        assert_eq!(
            decode_bits_with_erasures(&damaged, &erasures, input.len())
                .unwrap()
                .bytes,
            input
        );
    }

    #[test]
    fn lists_ambiguous_erasure_candidates_without_silently_accepting_one() {
        let input = b"bounded erasure recovery";
        let encoded = encode_bytes(input);
        let mut observed = vec![true; encoded.len()];
        for chunk in observed.chunks_mut(31) {
            for bit in &mut chunk[..7] {
                *bit = false;
            }
        }
        let candidates =
            erasure_candidates(&encoded, &observed, &vec![1.0; encoded.len()], input.len());
        assert!(candidates.iter().any(|candidate| candidate.bytes == input));
        assert!(candidates.len() <= 256);
    }
}
