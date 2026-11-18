//! Verified raw-250 and plain raw-122 operations. Platform file commands stay in the host.
use crate::native_values::{assign_native_value, DatabaseAddress, DatabaseEffect, NativeValue};
use crate::runtime::NativeContext;

/// A18560 publishes this request; the host commits it after the native context pass.
/// Positions use the engine's half-tile units, not pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapTransferRequest {
    /// A literal -1 map operand retains the currently active map.
    pub map_id: Option<i32>,
    pub x_half: i32,
    pub y_half: i32,
    /// Invalid/omitted direction values preserve the player's existing direction.
    pub direction: Option<i32>,
    /// Preserve the complete native flags, including the transition high nibble.
    pub flags: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleRequest {
    MapTransfer(MapTransferRequest),
    Exit,
}

/// Parse verified lifecycle forms without changing scenes, execution, or platform state.
/// On an accepted map request the host calls `yield_once`; while busy it returns false.
/// On accepted exit the host calls `invalidate_execution` and closes at its boundary.
pub fn parse_lifecycle_request(
    ctx: &NativeContext<'_>,
) -> Result<Option<LifecycleRequest>, String> {
    let raw = ctx
        .instruction
        .raw
        .as_ref()
        .ok_or("native command lacks raw record")?;
    match raw.opcode {
        // A1B950 does not consume parameters before requesting shutdown.
        175 => Ok(Some(LifecycleRequest::Exit)),
        130 => {
            if !(4..=6).contains(&raw.args.len()) || !raw.strings.is_empty() {
                return Err("native teleport requires 4..6 numeric arguments".into());
            }
            let a = &raw.args;
            if a[0] != -1 {
                return Err(format!(
                    "native teleport selector {} requires an actor or database adapter",
                    a[0]
                ));
            }
            let flags = a.get(4).copied().unwrap_or(0);
            let factor = 2 - (flags & 1);
            let direction = a
                .get(5)
                .map(|n| ctx.number(*n))
                .transpose()?
                .filter(|n| (1..=9).contains(n) && *n != 5);
            Ok(Some(LifecycleRequest::MapTransfer(MapTransferRequest {
                map_id: if a[3] == -1 {
                    None
                } else {
                    Some(ctx.number(a[3])?)
                },
                x_half: ctx.number(a[1])?.wrapping_mul(factor),
                y_half: ctx.number(a[2])?.wrapping_mul(factor),
                direction,
                flags,
            })))
        }
        _ => Ok(None),
    }
}

fn string_operand(ctx: &NativeContext<'_>, n: i32) -> bool {
    (3_000_000..=3_999_999).contains(&n)
        || (9_900_000..=9_999_999).contains(&n)
        || ((15_000_000..=15_999_999).contains(&n) && (5..=9).contains(&(n % 100)))
        || (ctx.instruction.source.page.is_none() && (1_600_005..=1_600_009).contains(&n))
}

fn read_value(ctx: &NativeContext<'_>, n: i32) -> Result<NativeValue, String> {
    if let Some(at) = DatabaseAddress::decode(n) {
        return ctx.world.databases.get(at).cloned();
    }
    if string_operand(ctx, n) {
        ctx.text(n).map(NativeValue::Text)
    } else {
        ctx.number(n).map(NativeValue::Number)
    }
}

fn write_value(
    ctx: &mut NativeContext<'_>,
    destination: i32,
    value: NativeValue,
) -> Result<(), String> {
    match value {
        NativeValue::Number(n) => ctx.set_number(destination, n),
        NativeValue::Text(s) => ctx.set_text(destination, s),
    }
}

/// Returns false only for opcodes/modes owned by another host service. Unsupported value
/// forms return a diagnostic and cannot silently become successful empty operations.
pub fn execute_values(ctx: &mut NativeContext<'_>) -> Result<bool, String> {
    let raw = ctx
        .instruction
        .raw
        .as_ref()
        .ok_or("native command lacks raw record")?;
    if raw.opcode == 250 {
        let strings = raw
            .strings
            .iter()
            .map(|s| s.text().map(str::to_owned).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let effect = ctx.world.databases.prepare_command(
            &raw.args,
            &strings,
            |n| read_value(ctx, n),
            |s| ctx.expand(s),
        )?;
        match effect {
            DatabaseEffect::Write { .. } | DatabaseEffect::Reset { .. } => ctx.world.databases.apply_write(effect)?,
            DatabaseEffect::Read {
                destination,
                value,
                operator,
            } => {
                let old = read_value(ctx, destination)?;
                write_value(ctx, destination, assign_native_value(old, value, operator)?)?;
            }
        }
        return Ok(true);
    }
    if raw.opcode != 122 {
        return Ok(false);
    }
    let flags = *raw.args.get(1).ok_or("string command lacks flags")? as u32;
    let operation = flags & 0x40F00;
    if matches!(operation, 0x500 | 0x600 | 0x40200) || flags & 0xf == 3 {
        return Ok(false);
    }
    if flags & !0xF1F != 0
        || !matches!(
            operation,
            0 | 0x100 | 0x200 | 0x300 | 0x400 | 0x800 | 0x900 | 0xA00 | 0xB00
        )
    {
        return Err(format!("unsupported native string flags {flags:#x}"));
    }
    let destination = if flags & 0x10 != 0 {
        ctx.number(raw.args[0])?
    } else {
        raw.args[0]
    };
    let source = match flags & 0xF {
        0 => ctx.expand(
            raw.strings
                .first()
                .map(|s| s.text())
                .transpose()
                .map_err(|e| e.to_string())?
                .unwrap_or(""),
        )?,
        1 => ctx.text(*raw.args.get(2).ok_or("string variable source missing")?)?,
        2 => {
            let n = *raw.args.get(2).ok_or("indirect string source missing")?;
            ctx.text(if string_operand(ctx, n) {
                n
            } else {
                ctx.number(n)?
            })?
        }
        _ => return Err("unsupported native string source mode".into()),
    };
    let old = if matches!(operation, 0 | 0x200 | 0x300 | 0x400) {
        String::new()
    } else {
        ctx.text(destination)?
    };
    let replacement = if operation == 0x900 {
        Some(
            ctx.expand(
                raw.strings
                    .get(1)
                    .ok_or("replace command lacks replacement")?
                    .text()
                    .map_err(|e| e.to_string())?,
            )?,
        )
    } else {
        None
    };
    let result = if matches!(operation, 0x200 | 0x300 | 0x400) {
        let (head, tail) = if operation == 0x400 {
            split_native_character(&source)?
        } else {
            split_native_line(&source)
        };
        if operation == 0x300 || operation == 0x400 && flags & 0xF != 0 {
            let operand = *raw
                .args
                .get(2)
                .ok_or("consume-line source operand missing")?;
            let source_address = match flags & 0xF {
                1 => operand,
                2 => {
                    if string_operand(ctx, operand) {
                        operand
                    } else {
                        ctx.number(operand)?
                    }
                }
                _ => return Err("consume-line needs a variable source".into()),
            };
            ctx.set_text(source_address, tail.to_owned())?;
        }
        head.to_owned()
    } else {
        transform_native_string(operation, &old, &source, replacement.as_deref())?
    };
    if string_operand(ctx, destination) {
        ctx.set_text(destination, result)?;
    } else {
        ctx.set_number(destination, parse_native_integer(&result)?)?;
    }
    Ok(true)
}

/// A11ADC..A11B67 uses UTF-8 leading-byte width (1..4), bounded by length.
/// Native byte-1 packets are consumed as a unit; this textual storage profile
/// diagnoses controls rather than treating their spelling as individual glyphs.
pub fn split_native_character(source: &str) -> Result<(&str, &str), String> {
    if source.starts_with('\u{1}') || source.starts_with('\\') {
        return Err("consume-character needs native control-token encoding".into());
    }
    let width = source.chars().next().map_or(0, char::len_utf8);
    Ok(source.split_at(width))
}

/// Native A126D9..A12F64 substring operations, restricted to text without internal controls.
pub fn transform_plain_string(
    operation: u32,
    old: &str,
    source: &str,
    replacement: Option<&str>,
) -> Result<String, String> {
    if old.contains(['\\', '\u{1}']) || source.contains(['\\', '\u{1}']) {
        return Err("native string controls require a separate transform profile".into());
    }
    transform_native_string(operation, old, source, replacement)
}

/// Source-preserving assignments and tag replacement used by Common89. This is not
/// a byte-indexed native control stream: substring slicing of controls stays rejected.
pub fn transform_native_string(
    operation: u32,
    old: &str,
    source: &str,
    replacement: Option<&str>,
) -> Result<String, String> {
    const LIMIT: usize = 16 * 1024 * 1024;
    if old.len() > LIMIT || source.len() > LIMIT || replacement.is_some_and(|s| s.len() > LIMIT) {
        return Err("native string input byte limit".into());
    }
    // Opcode 122/0x900 is the native tag replacement path used by Common89.
    // Its operands may already contain byte-1 display-control packets produced
    // by an earlier text command; replacement is byte-preserving and must not
    // discard those packets. Substring operations still reject control streams
    // because their native implementation indexes plain text bytes only.
    if (!matches!(operation, 0x900)
        && (old.contains('\u{1}')
            || source.contains('\u{1}')
            || replacement.is_some_and(|s| s.contains('\u{1}'))))
        || (matches!(operation, 0xA00 | 0xB00) && (old.contains('\\') || source.contains('\\')))
    {
        return Err("native string controls require a separate transform profile".into());
    }
    let result = match operation {
        0 => source.to_owned(),
        0x100 => {
            if old.len().saturating_add(source.len()) > LIMIT {
                return Err("native string result byte limit".into());
            }
            format!("{old}{source}")
        }
        0x800 => {
            if source.is_empty() {
                return Err("empty native removal pattern".into());
            }
            // A12740 restarts its search at zero after each removal, including new boundary matches.
            let mut value = old.to_owned();
            while let Some(at) = value.find(source) {
                value.replace_range(at..at + source.len(), "");
            }
            value
        }
        0x900 => {
            if source.is_empty() {
                return Err("empty native replacement pattern".into());
            }
            let replacement = replacement.ok_or("missing replacement text")?;
            let count = old.matches(source).count();
            let length = old.len() - count * source.len();
            if count
                .checked_mul(replacement.len())
                .and_then(|n| n.checked_add(length))
                .is_none_or(|n| n > LIMIT)
            {
                return Err("native string replacement byte limit".into());
            }
            old.replace(source, replacement)
        }
        0xA00 => old
            .find(source)
            .map(|at| old[..at].to_owned())
            .unwrap_or_else(|| "<<NotFound>>".into()),
        0xB00 => old
            .find(source)
            .map(|at| old[at + source.len()..].to_owned())
            .unwrap_or_else(|| "<<NotFound>>".into()),
        _ => return Err("unsupported plain-string operation".into()),
    };
    if result.len() > 16 * 1024 * 1024 {
        return Err("native string result byte limit".into());
    }
    Ok(result)
}

/// A11781..A11AD1: first LF ends the line; 0x300 writes its tail back to source.
pub fn split_native_line(source: &str) -> (&str, &str) {
    source.split_once('\n').unwrap_or((source, ""))
}

/// BA6B62/atoi behavior needed by text-to-numeric assignment; reject overflow explicitly.
pub(crate) fn parse_native_integer(source: &str) -> Result<i32, String> {
    let bytes = source
        .trim_start_matches([' ', '\t', '\n', '\r', '\u{b}', '\u{c}'])
        .as_bytes();
    let (negative, mut i) = match bytes.first() {
        Some(b'-') => (true, 1),
        Some(b'+') => (false, 1),
        _ => (false, 0),
    };
    let mut value = 0i64;
    while let Some(c @ b'0'..=b'9') = bytes.get(i) {
        value = value
            .checked_mul(10)
            .and_then(|v| v.checked_add((c - b'0') as i64))
            .ok_or("native atoi overflow")?;
        i += 1;
    }
    i32::try_from(if negative { -value } else { value }).map_err(|_| "native atoi overflow".into())
}
