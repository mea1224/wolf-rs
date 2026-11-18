//! Native A077D0 subsets whose device and mode semantics have been recovered.
pub fn validate_flags(flags: i32) -> Result<(), String> {
    if ((flags >> 16) & 15) > 3 {
        return Err("native input counter mode is not implemented".into());
    }
    match flags & 0xf00 {
        0 if flags & !0x300ff == 0 && flags & 15 <= 8 => Ok(()),
        0x200 if flags & !0x31381 == 0 => Ok(()),
        _ => Err("native input device/mode/flags outside verified subset".into()),
    }
}
