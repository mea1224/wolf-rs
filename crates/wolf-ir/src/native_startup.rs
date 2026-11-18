//! Verified ordinary-call operand binding, independent of editor metadata.
//!
//! Evidence: `validation/native/call-handlers.asm`, A1C03F..A1C0E2 (210),
//! and `../docs-append/evidence/rea-vm-a1cfa0.c`, A1D997..A1DB31 (300).
//! This module deliberately does not interpret string expansion,
//! reserved calls, dynamic targets, or the editor's argument labels/defaults.
use crate::{contextual_address, Source, Target, Text, Value, Var};
use wolf_format::RawCommand;

/// File header to native record +34/+38/+3C: 6A557A, 6A5A2A, 6A5EDA.
/// The trailing two bytes are preserved but do not participate in 958780 eligibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CommonCondition {
    pub flags: u8,
    pub encoded_lhs: i32,
    pub raw_rhs: i32,
    pub remaining: [u8; 2],
}

impl CommonCondition {
    pub fn decode(unknown1: u32, unknown2: &[u8]) -> Result<Self, String> {
        if unknown2.len() != 7 {
            return Err("native common condition requires the exact 11-byte header".into());
        }
        let mut header = [0; 11];
        header[..4].copy_from_slice(&unknown1.to_le_bytes());
        header[4..].copy_from_slice(unknown2);
        Ok(Self {
            flags: header[0],
            encoded_lhs: i32::from_le_bytes(header[1..5].try_into().unwrap()),
            raw_rhs: i32::from_le_bytes(header[5..9].try_into().unwrap()),
            remaining: [header[9], header[10]],
        })
    }

    /// 958780 resolves only the left operand; the right side remains a literal.
    /// `common_id` is the scheduler's common index, never the calling map's owner.
    /// `read` implements the native numeric getter on the relocated absolute code.
    pub fn eligible(
        &self,
        common_id: u32,
        mut read: impl FnMut(i32) -> Result<i32, String>,
    ) -> Result<bool, String> {
        match self.flags & 15 {
            3 => return Ok(true),
            2 => {}
            _ => return Ok(false),
        }
        let encoded = if (1_600_000..=1_699_999).contains(&self.encoded_lhs) {
            common_id
                .checked_mul(100)
                .and_then(|n| n.checked_add(self.encoded_lhs as u32 % 100))
                .filter(|&n| n < 1_000_000)
                .map(|n| 15_000_000 + n as i32)
                .ok_or("native common condition self address limit")?
        } else {
            self.encoded_lhs
        };
        let left = read(encoded)?;
        let right = self.raw_rhs;
        Ok(match self.flags >> 4 {
            0 => left > right,
            1 => left >= right,
            2 => left == right,
            3 => left <= right,
            4 => left < right,
            5 => left != right,
            6 => left & right == right,
            _ => false,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumericCommonCall {
    pub target: Target,
    /// Resolve and store these one at a time, in order, in the caller context.
    /// Resolving all operands before storing is wrong for aliasing self calls.
    pub arguments: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommonCall {
    pub target: Target,
    pub numbers: Vec<Value>,
    /// These are source expressions, not final argument values. Native literal
    /// text passes through 94FC50; references may trigger lazy expansion/writeback.
    /// A consumer must supply those semantics before entering the callee.
    pub strings: Vec<Text>,
    /// Raw caller destination; the callee metadata determines number/string kind.
    pub output: Option<i32>,
}

/// Decode ordinary 210/300 call operands. This describes the wire layout only;
/// it does not authorize treating unexpanded text as a final value.
pub fn common_call(raw: &RawCommand, source: &Source) -> Result<CommonCall, String> {
    if raw.extension_length != 0
        || !raw.extension.is_empty()
        || raw.movement.is_some()
        || raw.terminator != 0
    {
        return Err("unverified common-call envelope".into());
    }
    let [selector, flags, rest @ ..] = raw.args.as_slice() else {
        return Err("common call requires selector and flags".into());
    };
    let numeric_count = (flags & 15) as usize;
    let string_count = ((flags >> 4) & 15) as usize;
    let literals = ((flags >> 12) & 31) as u32;
    let has_output = flags & 0x1000000 != 0;
    if flags & !0x101f0ff != 0
        || numeric_count > 5
        || string_count > 5
        || literals >> string_count != 0
        || rest.len() != numeric_count + string_count + usize::from(has_output)
    {
        return Err("unverified common-call flags or parameter count".into());
    }
    // Reference-only calls have no text placeholders beyond the name in 300.
    // Literal string i uses text[i+1], preserving holes for earlier references.
    let required_texts = if literals != 0 {
        1 + (u32::BITS - literals.leading_zeros()) as usize
    } else {
        usize::from(raw.opcode == 300)
    };
    let max_texts = if raw.opcode == 300 || string_count != 0 {
        1 + string_count
    } else {
        0
    };
    if raw.strings.len() < required_texts || raw.strings.len() > max_texts {
        return Err("unverified common-call string vector layout".into());
    }
    let target = match raw.opcode {
        210 if (500_000..510_000).contains(selector) => {
            Target::Id(format!("common/{}", selector - 500_000))
        }
        300 if *selector == 0 => {
            let name = raw.strings[0].text().map_err(|e| e.to_string())?;
            if name.is_empty() || name.contains('\\') || name.contains("<<") {
                return Err("common name requires unresolved expansion or lookup".into());
            }
            Target::Name(name.to_owned())
        }
        _ => return Err("unverified common call kind or target".into()),
    };
    let numbers = rest[..numeric_count]
        .iter()
        .map(|&raw| {
            if raw < 1_000_000 {
                Ok(Value::Int(raw))
            } else {
                contextual_address(raw, source).map(Value::Var)
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    let strings = (0..string_count)
        .map(|i| {
            if literals & (1 << i) != 0 {
                raw.strings[1 + i]
                    .text()
                    .map(|s| Text::Literal(s.to_owned()))
                    .map_err(|e| e.to_string())
            } else {
                stored_text_address(rest[numeric_count + i], source).map(Text::Var)
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(CommonCall {
        target,
        numbers,
        strings,
        output: has_output.then(|| rest[numeric_count + string_count]),
    })
}

/// Decode only native synchronous common calls with numeric arguments.
/// Numeric destinations are persistent target commonself slots 0 through 4.
pub fn numeric_common_call(raw: &RawCommand, source: &Source) -> Result<NumericCommonCall, String> {
    if raw.extension_length != 0
        || !raw.extension.is_empty()
        || raw.movement.is_some()
        || raw.terminator != 0
    {
        return Err("unverified common-call envelope".into());
    }
    let [selector, flags, rest @ ..] = raw.args.as_slice() else {
        return Err("common call requires selector and flags".into());
    };
    if !(0..=5).contains(flags) || rest.len() != *flags as usize {
        return Err("only numeric self0..4 common arguments are supported".into());
    }
    let target = match raw.opcode {
        210 if (500_000..510_000).contains(selector) && raw.strings.is_empty() => {
            Target::Id(format!("common/{}", selector - 500_000))
        }
        300 if *selector == 0 && raw.strings.len() == 1 => {
            let name = raw.strings[0].text().map_err(|e| e.to_string())?;
            if name.is_empty() || name.contains('\\') || name.contains("<<") {
                return Err("common name requires unresolved expansion or lookup".into());
            }
            Target::Name(name.to_owned())
        }
        _ => return Err("unverified common call kind or target".into()),
    };
    let arguments = rest
        .iter()
        .map(|&raw| {
            if raw < 1_000_000 {
                Ok(Value::Int(raw))
            } else {
                contextual_address(raw, source).map(Value::Var)
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(NumericCommonCall { target, arguments })
}

/// Build target slots after actual name/ID resolution. This does not clear any
/// other self slot, and omitted arguments retain their previous values.
pub fn numeric_common_parameter_slots(common_id: u32, count: usize) -> Result<Vec<Var>, String> {
    if common_id >= 10_000 || count > 5 {
        return Err("numeric common argument binding outside verified storage".into());
    }
    Ok((0..count)
        .map(|i| Var::Common((common_id * 100) as i32 + i as i32))
        .collect())
}

pub fn string_common_parameter_slots(common_id: u32, count: usize) -> Result<Vec<Var>, String> {
    if common_id >= 10_000 || count > 5 {
        return Err("string common argument binding outside verified storage".into());
    }
    Ok((0..count)
        .map(|i| Var::Common((common_id * 100) as i32 + 5 + i as i32))
        .collect())
}

/// String-space addressing from A058D0/A05660 and 9E2260. This intentionally
/// rejects map-entity/system string getters; callers must provide their native
/// getter behavior instead of treating every address as generic storage.
pub fn stored_text_address(raw: i32, source: &Source) -> Result<Var, String> {
    match raw {
        3_000_000..=3_999_999 => Ok(Var::Global(raw - 3_000_000)),
        1_600_000..=1_699_999 if (5..=9).contains(&(raw % 100)) && source.page.is_none() => {
            let id = source
                .event
                .checked_mul(100)
                .and_then(|v| v.checked_add(raw as u32 % 100))
                .filter(|&v| v < 1_000_000)
                .ok_or("common string address limit")?;
            Ok(Var::Common(id as i32))
        }
        15_000_000..=15_999_999 if (5..=9).contains(&(raw % 100)) => {
            Ok(Var::Common(raw - 15_000_000))
        }
        _ => Err(format!("unverified stored string address {raw}")),
    }
}
