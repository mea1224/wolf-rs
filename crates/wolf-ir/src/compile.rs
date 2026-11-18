use super::*;
#[derive(Default, Debug, Serialize)]
pub struct OpcodeCoverage {
    pub total: usize,
    pub lowered: usize,
    /// Subset of lowered records deferred to host parameter validation and execution.
    pub native_host: usize,
    pub inert: usize,
    pub unsupported: usize,
}
#[derive(Default, Debug, Serialize)]
pub struct Coverage {
    pub total: usize,
    pub lowered: usize,
    /// Routing a raw command to a host is not evidence that its parameter form is supported.
    pub native_host: usize,
    pub supported: usize,
    pub inert: usize,
    pub failed_commands: usize,
    pub unsupported: BTreeMap<u32, usize>,
    pub reasons: BTreeMap<String, usize>,
    pub by_opcode: BTreeMap<u32, OpcodeCoverage>,
}
impl Coverage {
    pub fn add(&mut self, p: &Program) {
        for i in &p.code {
            self.total += 1;
            let opcode = i.raw.as_ref().map(|r| r.opcode);
            let record = opcode.map(|opcode| self.by_opcode.entry(opcode).or_default());
            if matches!(i.op, Op::NativeHost) {
                self.native_host += 1;
            }
            if let Some(record) = record {
                record.total += 1;
                if matches!(i.op, Op::NativeHost) {
                    record.native_host += 1;
                }
                match &i.op {
                    Op::Unsupported { .. } => record.unsupported += 1,
                    Op::Nop => record.inert += 1,
                    _ => record.lowered += 1,
                }
            }
            if let Op::Unsupported { opcode, reason } = &i.op {
                *self.unsupported.entry(*opcode).or_default() += 1;
                *self.reasons.entry(reason.clone()).or_default() += 1;
            } else {
                self.supported += 1;
                if matches!(i.op, Op::Nop) {
                    self.inert += 1;
                } else {
                    self.lowered += 1;
                }
            }
        }
    }
}
fn text(c: &RawCommand, n: usize) -> Result<String, String> {
    c.strings
        .get(n)
        .ok_or("missing string")?
        .text()
        .map(str::to_owned)
        .map_err(|e| e.to_string())
}
#[derive(Clone)]
struct Block {
    start: usize,
    indent: u8,
    markers: Vec<usize>,
    loop_block: bool,
}
/// Lowers only recognized parameter forms. All unrecognized forms retain a trapping raw instruction.
pub fn compile(
    id: String,
    name: String,
    source: Source,
    raw: &[RawCommand],
) -> Result<Program, String> {
    let address = |n| super::contextual_address(n, &source);
    let value = |n| {
        if n < 1_000_000 {
            Ok(Value::Int(n))
        } else if matches!(n,1_000_000_000..=1_199_999_999|1_300_000_000..=1_399_999_999) {
            Ok(Value::Database(n))
        } else {
            address(n).map(Value::Var)
        }
    };
    let common_event = source.page.is_none().then_some(source.event);
    let arithmetic_value = |n| native_arithmetic_value(n, common_event);
    let mut ends = BTreeMap::new();
    let mut markers: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut breaks = BTreeMap::new();
    let mut stack: Vec<Block> = Vec::new();
    let mut labels: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut uncertain_labels = Vec::new();
    let mut errors = BTreeMap::new();
    for (i, c) in raw.iter().enumerate() {
        match c.opcode {
            // 176 is not a block opener in this corpus: 170+179 pair with all 1068 ends.
            102 | 111 | 112 | 170 | 179 => stack.push(Block {
                start: i,
                indent: c.indent,
                markers: Vec::new(),
                loop_block: matches!(c.opcode, 170 | 179),
            }),
            401 | 402 | 420 | 421 => {
                if let Some(b) = stack
                    .last_mut()
                    .filter(|b| !b.loop_block && b.indent == c.indent)
                {
                    b.markers.push(i);
                } else {
                    errors.insert(i, "orphan or mismatched branch marker".to_owned());
                }
            }
            498 | 499 => {
                let Some(pos) = stack
                    .iter()
                    .rposition(|b| b.indent == c.indent && b.loop_block == (c.opcode == 498))
                else {
                    // A damaged/unrecognized end must not consume an unrelated outer block.
                    // Native 499 only resets branch_state[indent], even without an opener.
                    if c.opcode == 498 {
                        errors.insert(i, "orphan or mismatched loop end".to_owned());
                    }
                    continue;
                };
                for b in stack.drain(pos + 1..) {
                    errors.insert(b.start, "unclosed nested control block".to_owned());
                    for m in b.markers {
                        errors.insert(m, "unclosed nested control block".to_owned());
                    }
                }
                let b = stack.pop().unwrap();
                ends.insert(b.start, i);
                if b.loop_block {
                    ends.insert(i, b.start);
                } else {
                    markers.insert(b.start, b.markers);
                }
            }
            171 | 176 => {
                if let Some(b) = stack.iter().rev().find(|b| b.loop_block) {
                    breaks.insert(i, b.start);
                }
            }
            212 if c.args.is_empty()
                && c.strings.len() == 1
                && c.extension_length == 0
                && c.extension.is_empty()
                && c.movement.is_none()
                && c.terminator == 0 =>
            {
                match text(c, 0) {
                    Ok(label) if !label.contains('\\') => labels.entry(label).or_default().push(i),
                    Ok(_) => uncertain_labels.push(i),
                    Err(e) => {
                        uncertain_labels.push(i);
                        errors.insert(i, e);
                    }
                }
            }
            212 => uncertain_labels.push(i),
            _ => {}
        }
    }
    for b in stack {
        errors.insert(b.start, "unclosed control block".to_owned());
        for m in b.markers {
            errors.insert(m, "unclosed control block".to_owned());
        }
    }
    let mut marker_ends = BTreeMap::new();
    for (start, list) in &markers {
        for m in list {
            marker_ends.insert(*m, ends[start] + 1);
        }
    }
    let mut code = Vec::with_capacity(raw.len());
    for (i, c) in raw.iter().enumerate() {
        let a = &c.args;
        let lower = || -> Result<Op, String> {
            if let Some(reason) = errors.get(&i) {
                return Err(reason.clone());
            }
            if c.extension_length != 0 || !c.extension.is_empty() {
                return Err("unknown command extension".into());
            }
            if (c.movement.is_some() && c.opcode != 201)
                || c.terminator != if c.opcode == 201 { 1 } else { 0 }
            {
                return Err("movement/terminator semantics require a route profile".into());
            }
            match c.opcode {
                // Static dispatcher cases for these opcodes go directly to A09386:
                // no argument or text is read. Envelope/extension checks above still apply.
                0 | 99 | 103 => Ok(Op::Nop),
                106 | 122 | 124 | 130 | 140 | 150 | 160 | 161 | 162 | 175 | 220 | 221 | 222
                | 151 | 173 | 201 | 202 | 230 | 231 | 242 | 250 | 270 | 281 | 290 | 1000 => Ok(Op::NativeHost),
                121 if (4..=5).contains(&a.len()) && c.strings.is_empty() => {
                    let flags = a[3];
                    if flags & !0x1ff7d != 0 {
                        return Err("float/indirect operand flags".into());
                    }
                    let ranged = flags & 0x10000 != 0;
                    if a.len() != if ranged { 5 } else { 4 } {
                        return Err("numeric destination range argument count".into());
                    }
                    // Update9 consumes original lhs/rhs, bypassing the arithmetic
                    // scratch result (A0F8F6). Native scripts encode binary15 here.
                    let binary = match ((flags >> 8) & 15, (flags >> 12) & 15) {
                        (9, 15) => Binary::NativeAngle,
                        (9, _) => return Err("angle assignment with arithmetic side effects is unverified".into()),
                        (_, 0) => Binary::Add,
                        (_, 1) => Binary::Sub,
                        (_, 2) => Binary::Mul,
                        (_, 3) => Binary::Div,
                        (_, 4) => Binary::Rem,
                        (_, 5) => Binary::And,
                        (_, 6) => Binary::RandomInclusive,
                        (_, 7) => Binary::Or,
                        (_, 8) => Binary::Xor,
                        (_, 9) => Binary::SignedShift,
                        _ => return Err("unverified arithmetic operator".into()),
                    };
                    let update = match (flags >> 8) & 15 {
                        0 => Update::Set,
                        1 => Update::Add,
                        2 => Update::Sub,
                        3 => Update::Mul,
                        4 => Update::Div,
                        5 => Update::Rem,
                        6 => Update::Max,
                        7 => Update::Min,
                        8 => Update::Abs,
                        9 => Update::Set,
                        10 => Update::NativeSine,
                        11 => Update::NativeCosine,
                        12 => Update::NativeSqrt,
                        _ => return Err("unverified assignment operator".into()),
                    };
                    let mut lhs = if flags & 4 != 0 {
                        Value::Int(a[1])
                    } else {
                        arithmetic_value(a[1])?
                    };
                    let mut rhs = if flags & 8 != 0 {
                        Value::Int(a[2])
                    } else {
                        arithmetic_value(a[2])?
                    };
                    if flags & 0x20 != 0 { lhs = Value::NativeIndirect { pointer: Box::new(lhs), common_event }; }
                    if flags & 0x40 != 0 { rhs = Value::NativeIndirect { pointer: Box::new(rhs), common_event }; }
                    let arithmetic = Arithmetic::WolfInteger {
                        clamp_999999: flags & 1 != 0,
                    };
                    if flags & 0x10 != 0 {
                        if ranged {
                            return Err("indirect destination ranges are unverified".into());
                        }
                        Ok(Op::ComputeIndirect {
                            pointer: value(a[0])?,
                            lhs,
                            binary,
                            rhs,
                            update,
                            arithmetic,
                        })
                    } else if ranged {
                        let maximum = a[4].max(0);
                        if maximum >= 10_000 {
                            return Err("numeric destination range limit".into());
                        }
                        let destinations = (0..=maximum)
                            .filter_map(|offset| {
                                let raw = a[0]
                                    .checked_add(offset)
                                    .ok_or("numeric destination overflow");
                                match raw {
                                    // A0EF75/A101B1 skips string destinations in
                                    // a nonzero numeric range, preserving text.
                                    Ok(raw) if maximum > 0 && contextual_string_address(raw, common_event).is_ok() => None,
                                    Ok(raw) => Some(address(raw)),
                                    Err(error) => Some(Err(error.into())),
                                }
                            })
                            .collect::<Result<Vec<_>, String>>()?;
                        if destinations.is_empty() { return Ok(Op::Nop); }
                        Ok(Op::ComputeRange {
                            destinations,
                            lhs,
                            binary,
                            rhs,
                            update,
                            arithmetic,
                        })
                    } else {
                        Ok(Op::Compute {
                            dst: address(a[0])?,
                            lhs,
                            binary,
                            rhs,
                            update,
                            arithmetic,
                        })
                    }
                }
                111 if !a.is_empty()
                    && a[0] & !0x1f == 0
                    && (1..=4).contains(&(a[0] & 15))
                    && a.len() == 1 + 3 * (a[0] & 15) as usize
                    && c.strings.is_empty() =>
                {
                    let mut cases = Vec::new();
                    let list = markers.get(&i).ok_or("unbound condition block")?;
                    // A0D052/A0D055 use only the count word's low nibble.
                    for n in 0..(a[0] & 15) as usize {
                        let flags = a[3 + n * 3];
                        if flags & !0x1f != 0 {
                            return Err("unknown numeric condition flags".into());
                        }
                        let mut matches = list
                            .iter()
                            .filter(|m| raw[**m].opcode == 401 && raw[**m].args == [n as i32 + 1]);
                        let marker = *matches.next().ok_or("missing numbered condition branch")?;
                        if matches.next().is_some() {
                            return Err("ambiguous numbered condition branch".into());
                        }
                        let lhs = value(a[1 + n * 3])?;
                        // GamePro 0xa0d171: bit 0x10 bypasses RHS variable resolution.
                        let rhs = if flags & 0x10 != 0 {
                            Value::Int(a[2 + n * 3])
                        } else {
                            value(a[2 + n * 3])?
                        };
                        let condition = if flags & 15 == 6 {
                            Condition::BitsContain { lhs, rhs }
                        } else {
                            let cmp = match flags & 15 {
                                0 => Compare::Gt,
                                1 => Compare::Ge,
                                2 => Compare::Eq,
                                3 => Compare::Le,
                                4 => Compare::Lt,
                                5 => Compare::Ne,
                                _ => return Err("unknown numeric comparison mode".into()),
                            };
                            Condition::Number { lhs, cmp, rhs }
                        };
                        cases.push((condition, marker + 1));
                    }
                    let mut defaults = list.iter().filter(|m| raw[**m].opcode == 420);
                    let fallback = defaults
                        .next()
                        .map_or(*ends.get(&i).ok_or("unbound block end")? + 1, |m| *m + 1);
                    if defaults.next().is_some() {
                        return Err("ambiguous default condition branch".into());
                    }
                    Ok(Op::Branch { cases, fallback })
                }
                112 if !a.is_empty() && a[0] & !0x1f == 0 && (1..=4).contains(&(a[0] & 15)) => {
                    let count = (a[0] & 15) as usize;
                    if ![1 + count, 1 + 2 * count].contains(&a.len())
                        || !(count..=4).contains(&c.strings.len())
                    {
                        return Err("invalid string condition operand counts".into());
                    }
                    let common_event = source.page.is_none().then_some(source.event);
                    let list = markers.get(&i).ok_or("unbound condition block")?;
                    let mut cases = Vec::new();
                    for n in 0..count {
                        // A0D430 / A0D618 / A0DF32 split address, RHS mode, comparison.
                        let packed = a[1 + n] as u32;
                        let cmp = match packed >> 28 {
                            0 => StringCompare::Eq,
                            1 => StringCompare::Ne,
                            2 => StringCompare::Contains,
                            3 => StringCompare::StartsWith,
                            4 => StringCompare::NotContains,
                            5 => StringCompare::EndsWith,
                            _ => return Err("unsupported string comparison mode".into()),
                        };
                        let lhs = Text::Var(contextual_string_address(
                            (packed & 0x00ff_ffff) as i32,
                            common_event,
                        )?);
                        let rhs = match (packed >> 24) & 15 {
                            0 => Text::Literal(text(c, n)?),
                            1 => Text::Var(contextual_string_address(
                                *a.get(1 + count + n).ok_or("missing string RHS address")?,
                                common_event,
                            )?),
                            _ => return Err("unsupported string RHS mode".into()),
                        };
                        let mut matches = list
                            .iter()
                            .filter(|m| raw[**m].opcode == 401 && raw[**m].args == [n as i32 + 1]);
                        let marker = *matches.next().ok_or("missing numbered condition branch")?;
                        if matches.next().is_some() {
                            return Err("ambiguous numbered condition branch".into());
                        }
                        cases.push((
                            Condition::StringCompare {
                                lhs,
                                cmp,
                                rhs,
                                common_event,
                            },
                            marker + 1,
                        ));
                    }
                    let mut defaults = list.iter().filter(|m| raw[**m].opcode == 420);
                    let fallback = defaults
                        .next()
                        .map_or(*ends.get(&i).ok_or("unbound block end")? + 1, |m| *m + 1);
                    if defaults.next().is_some() {
                        return Err("ambiguous default condition branch".into());
                    }
                    Ok(Op::Branch { cases, fallback })
                }
                101 if a.is_empty() && c.strings.len() == 1 => {
                    Ok(Op::Request(Service::Message { text: text(c, 0)? }))
                }
                170 if a.is_empty() => Ok(Op::Loop {
                    // Native pushes two billion repeats after the first body pass.
                    count: Some(Value::Int(2_000_000_001)),
                    end: *ends.get(&i).ok_or("unbound loop end")? + 1,
                }),
                179 if a.len() == 1 => Ok(Op::Loop {
                    count: Some(value(a[0])?),
                    end: *ends.get(&i).ok_or("unbound loop end")? + 1,
                }),
                171 if a.is_empty() => {
                    let start = *breaks.get(&i).ok_or("break outside loop")?;
                    Ok(Op::Break {
                        start,
                        end: *ends.get(&start).ok_or("unbound break exit")? + 1,
                    })
                }
                176 if a.is_empty() => {
                    let start = breaks.get(&i).ok_or("continue outside loop")?;
                    Ok(Op::Jump(*ends.get(start).ok_or("unbound continue exit")?))
                }
                172 if a.is_empty() => Ok(Op::Return),
                180 if a.len() == 1 => Ok(Op::NativeWait(value(a[0])?)),
                125 if a.len() == 1 && c.strings.is_empty() && a[0] & !0xf7 == 0 => {
                    Ok(Op::SimulateInput { flags: a[0] })
                }
                126 if a.len() == 1 && c.strings.is_empty() => {
                    if a[0] & !0x3f7 != 0 {
                        return Err("unsupported input control flags".into());
                    }
                    Ok(Op::InputControl { flags: a[0] })
                }
                123 if (2..=3).contains(&a.len()) && c.strings.is_empty() => {
                    let flags = a[1];
                    if flags & 0xf00 != 0x100 {
                        super::native_input::validate_flags(flags)?;
                        let dst = address(a[0])?;
                        if matches!(dst, Var::System(_)) {
                            return Err("input system-variable destination is unsupported".into());
                        }
                        return Ok(Op::NativeInput {
                            dst,
                            key: if flags & 0x1000 != 0 && a.len() == 3 {
                                value(a[2])?
                            } else {
                                Value::Int(0)
                            },
                            flags,
                        });
                    }
                    if flags & 0xf00 != 0x100 || flags & !0xf1181 != 0 || (flags >> 16) & 15 > 3 {
                        return Err(
                            "input device/mode/flags outside verified keyboard subset".into()
                        );
                    }
                    let dst = address(a[0])?;
                    if matches!(dst, Var::System(_)) {
                        return Err("keyboard system-variable destination is unsupported".into());
                    }
                    Ok(Op::Keyboard {
                        dst,
                        key: if flags & 0x1000 != 0 && a.len() == 3 {
                            value(a[2])?
                        } else {
                            Value::Int(0)
                        },
                        mode: ((flags >> 16) & 15) as u8,
                        wait: flags & 0x80 != 0,
                    })
                }
                210 | 300 => {
                    let call = super::native_startup::common_call(c, &source)?;
                    Ok(Op::NativeCommonCall {
                        target: call.target,
                        numbers: call.numbers,
                        strings: call.strings,
                        output: call.output,
                    })
                }
                211 if a.len() == 2 && a[1] == 0 && c.strings.is_empty() =>
                    Ok(Op::NativeReserve { target: value(a[0])? }),
                212 if a.is_empty() && c.strings.len() == 1 => Ok(Op::Nop),
                213 if a.is_empty() && c.strings.len() == 1 => {
                    let label = text(c, 0)?;
                    if label.contains('\\') {
                        return Err("label text expansion is unverified".into());
                    }
                    // GamePro A1E230 ordinary scan selects the first equal label.
                    let target = *labels
                        .get(&label)
                        .and_then(|p| p.first())
                        .ok_or("unresolved label")?;
                    // An earlier dynamic label may expand to the same literal target.
                    // First-match selection is not static until those candidates are known.
                    if uncertain_labels.first().is_some_and(|pc| *pc < target) {
                        return Err("earlier label expansion or form is unverified".into());
                    }
                    // Native cross-loop jumps rebuild loop bookkeeping. A plain IR jump
                    // only has verified behavior when both sites have identical ancestry.
                    if ends.iter().any(|(&start, &end)| {
                        matches!(raw[start].opcode, 170 | 179)
                            && (start < i && i <= end) != (start < target && target <= end)
                    }) {
                        let loops = ends.iter().filter_map(|(&start, &end)| {
                            (matches!(raw[start].opcode, 170 | 179) && start < target && target <= end).then_some(start)
                        }).collect();
                        return Ok(Op::NativeLabelJump { target, loops });
                    }
                    Ok(Op::Jump(target))
                }
                401 | 402 | 420 | 421 if a.len() == 1 => Ok(Op::Jump(
                    *marker_ends.get(&i).ok_or("unbound branch marker")?,
                )),
                498 if a.is_empty() => {
                    let start = *ends.get(&i).ok_or("unbound loop start")?;
                    Ok(Op::Next { start })
                }
                499 if a.is_empty() => Ok(Op::Nop),
                _ => Err("opcode or parameter form has no verified lowering".into()),
            }
        };
        let op = lower().unwrap_or_else(|reason| Op::Unsupported {
            opcode: c.opcode,
            reason,
        });
        let mut s = source.clone();
        s.command = source
            .command
            .checked_add(i)
            .ok_or("source command index overflow")?;
        s.span = [c.span.start, c.span.end];
        code.push(Instruction {
            source: s,
            raw: Some(c.clone()),
            op,
        });
    }
    // Keep malformed loop components as local traps, without losing the whole program.
    for pc in 0..code.len() {
        if let Op::Loop { end, .. } = code[pc].op {
            if !matches!(code[end - 1].op, Op::Next { start } if start == pc) {
                code[pc].op = Op::Unsupported {
                    opcode: raw[pc].opcode,
                    reason: "loop end has unsupported semantics".into(),
                };
            }
        }
    }
    for pc in 0..code.len() {
        let start = match code[pc].op {
            Op::Next { start } | Op::Break { start, .. } => start,
            Op::Jump(_) if raw[pc].opcode == 176 => breaks[&pc],
            _ => continue,
        };
        if !matches!(code[start].op, Op::Loop { .. }) {
            code[pc].op = Op::Unsupported {
                opcode: raw[pc].opcode,
                reason: "loop opener has unsupported semantics".into(),
            };
        }
    }
    let p = Program {
        id,
        name,
        profile: PROFILE.into(),
        code,
    };
    p.validate()?;
    Ok(p)
}
