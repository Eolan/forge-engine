//! A precise entry's mixes written out (#282). `-fp-mode precise` marks every multiply and add
//! `NoContraction`, but Slang compiles `lerp` to GLSL.std.450's `FMix`, which the mark does not
//! reach, and NVIDIA's two variants of a pipeline evaluated one `FMix` two ways (#281). Each
//! `FMix(a, b, t)` becomes `(b − a) · t + a` (`lerp_exact` in `shaders/exact.slang`): an `OpFSub`,
//! an `OpFMul` and an `OpFAdd`, each `NoContraction`, so the rule holds whatever the source says.

/// SPIR-V's magic number, the module's first word.
const MAGIC: u32 = 0x0723_0203;
/// The header's words: magic, version, generator, bound, schema.
const HEADER: usize = 5;

const OP_EXT_INST_IMPORT: u32 = 11;
const OP_EXT_INST: u32 = 12;
const OP_DECORATE: u32 = 71;
const OP_F_ADD: u32 = 129;
const OP_F_SUB: u32 = 131;
const OP_F_MUL: u32 = 133;
/// The annotations' opcodes: the section new decorations join.
const ANNOTATIONS: [u32; 8] = [71, 72, 73, 74, 75, 332, 5632, 5633];
/// The types' and constants' first opcodes (`OpTypeVoid` to `OpTypeForwardPointer`), where the
/// annotations end in a module without any.
const TYPES: std::ops::RangeInclusive<u32> = 19..=39;
/// GLSL.std.450's `FMix`.
const FMIX: u32 = 46;
/// The `NoContraction` decoration.
const NO_CONTRACTION: u32 = 42;

/// The instruction at `words[at]`: its opcode and its word count (at least 1).
fn instruction(words: &[u32], at: usize) -> (u32, usize) {
    (words[at] & 0xFFFF, ((words[at] >> 16) as usize).max(1))
}

/// Whether the literal string starting at `words[at..]` is `name` (nul-terminated, packed four
/// bytes a word, little-endian).
fn string_is(words: &[u32], at: usize, name: &str) -> bool {
    let bytes: Vec<u8> = words[at..]
        .iter()
        .flat_map(|w| w.to_le_bytes())
        .take_while(|&b| b != 0)
        .collect();
    bytes == name.as_bytes()
}

/// `words` with every `FMix` of GLSL.std.450 written out as `(b − a) · t + a`, each operation
/// `NoContraction`; the count of mixes rewritten. A module that is not SPIR-V, or has no mix, is
/// returned as it is.
pub(crate) fn exact_mixes(words: &[u32]) -> (Vec<u32>, usize) {
    if words.len() < HEADER || words[0] != MAGIC {
        return (words.to_vec(), 0);
    }
    // GLSL.std.450's id, and where the annotations end.
    let mut glsl = None;
    let mut last_annotation = None;
    let mut first_type = None;
    let mut at = HEADER;
    while at < words.len() {
        let (op, count) = instruction(words, at);
        if op == OP_EXT_INST_IMPORT && string_is(words, at + 2, "GLSL.std.450") {
            glsl = Some(words[at + 1]);
        } else if ANNOTATIONS.contains(&op) {
            last_annotation = Some(at + count);
        } else if TYPES.contains(&op) && first_type.is_none() {
            first_type = Some(at);
        }
        at += count;
    }
    let Some(glsl) = glsl else {
        return (words.to_vec(), 0);
    };
    let Some(decorations_at) = last_annotation.or(first_type) else {
        return (words.to_vec(), 0);
    };
    let mut bound = words[3];
    let mut body: Vec<u32> = Vec::with_capacity(words.len() + 64);
    let mut decorations: Vec<u32> = Vec::new();
    let mut mixes = 0;
    let mut at = decorations_at;
    while at < words.len() {
        let (op, count) = instruction(words, at);
        let w = &words[at..at + count];
        if op == OP_EXT_INST && count == 8 && w[3] == glsl && w[4] == FMIX {
            let (ty, result, a, b, t) = (w[1], w[2], w[5], w[6], w[7]);
            let (difference, scaled) = (bound, bound + 1);
            bound += 2;
            body.extend([OP_F_SUB | 5 << 16, ty, difference, b, a]);
            body.extend([OP_F_MUL | 5 << 16, ty, scaled, difference, t]);
            body.extend([OP_F_ADD | 5 << 16, ty, result, scaled, a]);
            for id in [difference, scaled, result] {
                decorations.extend([OP_DECORATE | 3 << 16, id, NO_CONTRACTION]);
            }
            mixes += 1;
        } else {
            body.extend_from_slice(w);
        }
        at += count;
    }
    if mixes == 0 {
        return (words.to_vec(), 0);
    }
    let mut out = Vec::with_capacity(decorations_at + decorations.len() + body.len());
    out.extend_from_slice(&words[..decorations_at]);
    out[3] = bound;
    out.extend(decorations);
    out.extend(body);
    (out, mixes)
}

/// How many `FMix` of GLSL.std.450 `words` holds.
pub(crate) fn mixes(words: &[u32]) -> usize {
    if words.len() < HEADER || words[0] != MAGIC {
        return 0;
    }
    let mut glsl = None;
    let mut found = 0;
    let mut at = HEADER;
    while at < words.len() {
        let (op, count) = instruction(words, at);
        if op == OP_EXT_INST_IMPORT && string_is(words, at + 2, "GLSL.std.450") {
            glsl = Some(words[at + 1]);
        } else if op == OP_EXT_INST
            && count == 8
            && Some(words[at + 3]) == glsl
            && words[at + 4] == FMIX
        {
            found += 1;
        }
        at += count;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "GLSL.std.450" packed as a literal string.
    fn glsl_name() -> Vec<u32> {
        let mut bytes = b"GLSL.std.450".to_vec();
        bytes.resize(16, 0);
        bytes
            .chunks(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect()
    }

    /// A module's skeleton: the header, GLSL.std.450 as %1, one decoration, `float` as %2, and
    /// `%10 = FMix %2 %11 %12 %13` in a body.
    fn module() -> Vec<u32> {
        let mut w = vec![MAGIC, 0x0001_0600, 0, 20, 0];
        w.push(OP_EXT_INST_IMPORT | 6 << 16);
        w.push(1);
        w.extend(glsl_name());
        w.extend([OP_DECORATE | 3 << 16, 5, 0]); // %5 RelaxedPrecision
        w.extend([22 | 3 << 16, 2, 32]); // %2 = OpTypeFloat 32
        w.extend([OP_EXT_INST | 8 << 16, 2, 10, 1, FMIX, 11, 12, 13]);
        w
    }

    #[test]
    fn a_mix_becomes_a_sub_a_mul_and_an_add_each_uncontracted() {
        let words = module();
        assert_eq!(mixes(&words), 1);
        let (out, count) = exact_mixes(&words);
        assert_eq!(count, 1);
        assert_eq!(mixes(&out), 0);
        assert_eq!(out[3], 22, "two new ids");
        let tail = &out[out.len() - 15..];
        assert_eq!(tail[..5], [OP_F_SUB | 5 << 16, 2, 20, 12, 11]);
        assert_eq!(tail[5..10], [OP_F_MUL | 5 << 16, 2, 21, 20, 13]);
        assert_eq!(tail[10..], [OP_F_ADD | 5 << 16, 2, 10, 21, 11]);
        // The decorations follow the module's own, before the types.
        let float_at = out
            .iter()
            .position(|&w| w == (22 | 3 << 16))
            .expect("the float type");
        let decorated: Vec<u32> = out[float_at - 9..float_at]
            .chunks(3)
            .map(|d| {
                assert_eq!([d[0], d[2]], [OP_DECORATE | 3 << 16, NO_CONTRACTION]);
                d[1]
            })
            .collect();
        assert_eq!(decorated, [20, 21, 10]);
    }

    #[test]
    fn a_module_without_mixes_is_left_alone() {
        let mut words = module();
        words.truncate(words.len() - 8);
        assert_eq!(exact_mixes(&words), (words.clone(), 0));
        assert_eq!(exact_mixes(&[1, 2, 3]).1, 0);
    }
}
