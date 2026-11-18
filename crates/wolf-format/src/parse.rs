use crate::{reader::Reader, *};

fn route(r: &mut Reader<'_>) -> Result<RouteCommand> {
    let start = r.pos;
    let opcode = r.u8()?;
    let count = r.u8()? as usize;
    let mut args = r.vec_capacity(count, 4)?;
    for _ in 0..count {
        args.push(r.i32()?);
    }
    r.expect(&[1, 0])?;
    Ok(RouteCommand {
        span: r.span(start),
        opcode,
        args,
    })
}
fn command(r: &mut Reader<'_>) -> Result<RawCommand> {
    let start = r.pos;
    let words = r.u8()? as usize;
    if words == 0 {
        return Err(r.error("zero command word count"));
    }
    let opcode = r.u32()?;
    let mut args = r.vec_capacity(words - 1, 4)?;
    for _ in 1..words {
        args.push(r.i32()?);
    }
    let indent = r.u8()?;
    let strings_count = r.u8()? as usize;
    let mut strings = r.vec_capacity(strings_count, 5)?;
    for _ in 0..strings_count {
        strings.push(r.string()?);
    }
    let terminator = r.u8()?;
    let movement = if terminator == 1 || opcode == 201 {
        Some(MoveData {
            unknown: r.blob(5)?,
            flags: r.u8()?,
            routes: r.vector(4, route)?,
        })
    } else {
        if terminator != 0 {
            return Err(r.error("unknown command terminator"));
        }
        None
    };
    let extension_length = r.u8()?;
    let extension = r.blob(extension_length as usize)?;
    Ok(RawCommand {
        span: r.span(start),
        opcode,
        args,
        indent,
        strings,
        terminator,
        movement,
        extension_length,
        extension,
    })
}
fn page(r: &mut Reader<'_>) -> Result<Page> {
    let start = r.pos;
    let unknown = r.u32()?;
    let graphic = r.string()?;
    let graphic_fields = r.blob(4)?;
    let conditions = r.blob(37)?;
    let movement = r.blob(4)?;
    let flags = r.u8()?;
    let route_flags = r.u8()?;
    let routes = r.vector(4, route)?;
    let commands = r.vector(9, command)?;
    let features = r.u32()?;
    let shadow_collision = r.blob(3)?;
    let page_transfer = if features > 3 { Some(r.u8()?) } else { None };
    r.expect(&[0x7a])?;
    Ok(Page {
        span: r.span(start),
        unknown,
        graphic,
        graphic_fields,
        conditions,
        movement,
        flags,
        route_flags,
        routes,
        commands,
        features,
        shadow_collision,
        page_transfer,
    })
}
fn event(r: &mut Reader<'_>) -> Result<MapEvent> {
    let start = r.pos;
    r.expect(&[0x6f, 0x39, 0x30, 0, 0])?;
    let id = r.u32()?;
    let name = r.string()?;
    let x = r.u32()?;
    let y = r.u32()?;
    let count = r.count()?;
    r.expect(&[0, 0, 0, 0])?;
    let mut pages = r.vec_capacity(count, 1)?;
    for _ in 0..count {
        r.expect(&[0x79])?;
        pages.push(page(r)?);
    }
    r.expect(&[0x70])?;
    Ok(MapEvent {
        span: r.span(start),
        id,
        name,
        x,
        y,
        pages,
    })
}
pub fn parse_map(input: &[u8], limits: &Limits) -> Result<Parsed<Map>> {
    let mut header = Reader::new(input, 0, Domain::Stored, limits)?;
    header.expect(&[0; 10])?;
    header.expect(b"WOLFM\0")?;
    header.expect(&[0x55, 0, 0, 0])?;
    let version = header.u32()?;
    if !matches!(version, 100 | 103) {
        return Err(header.error("unsupported map version; expected 100 or 103"));
    }
    let unknown_byte_24 = header.u8()?;
    let reconstructed = if version == 103 {
        lz4::reconstruct(input, 25, limits)?
    } else {
        if input.len() > limits.max_stored_bytes || input.len() > limits.max_decoded_bytes {
            return Err(header.error("uncompressed map size limit"));
        }
        input.to_vec()
    };
    let mut r = Reader::new(&reconstructed, 25, Domain::Reconstructed, limits)?;
    let label = r.string()?;
    let tileset = r.u32()?;
    let width = r.u32()?;
    let height = r.u32()?;
    let count = r.count()?;
    let unknown = if version == 103 { Some(r.u32()?) } else { None };
    let layers = if version == 103 { r.u32()? } else { 3 };
    if layers != 3 {
        return Err(r.error("map103 profile requires 3 layers"));
    }
    // The only observed v100 file has no events. Its command extension grammar
    // differs from v103; do not accidentally parse those commands as v103.
    if version == 100 && count != 0 {
        return Err(r.error("map100 events are not yet supported"));
    }
    let words = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(layers as usize))
        .ok_or_else(|| r.error("tile dimension overflow"))?;
    if width == 0 || height == 0 || words > limits.max_tile_words {
        return Err(r.error("tile dimension limit"));
    }
    let tiles_start = r.pos;
    let tiles = if r.bytes.get(r.pos..r.pos + 4) == Some(&[0xff; 4]) {
        r.take(4)?;
        None
    } else {
        let mut values = r.vec_capacity(words, 4)?;
        for _ in 0..words {
            values.push(r.i32()?);
        }
        Some(values)
    };
    let tiles_span = r.span(tiles_start);
    let mut events = r.vec_capacity(count, 1)?;
    for _ in 0..count {
        events.push(event(&mut r)?);
    }
    r.expect(&[0x66])?;
    r.eof()?;
    Ok(Parsed {
        value: Map {
            version,
            unknown_byte_24,
            label,
            tileset,
            width,
            height,
            unknown,
            layers,
            tiles,
            tiles_span,
            events,
        },
        reconstructed,
    })
}
fn common_event(r: &mut Reader<'_>) -> Result<CommonEvent> {
    let start = r.pos;
    r.expect(&[0x8e])?;
    let id = r.u32()?;
    let unknown1 = r.u32()?;
    let unknown2 = r.blob(7)?;
    let name = r.string()?;
    let commands = r.vector(9, command)?;
    let unknown11 = r.string()?;
    let description = r.string()?;
    r.expect(&[0x8f])?;
    let unknown3 = r.vector(5, Reader::string)?;
    let n = r.count()?;
    let unknown4 = r.blob(n)?;
    let unknown5 = r.vector(4, |r| r.vector(5, Reader::string))?;
    let unknown6 = r.vector(4, |r| r.vector(4, Reader::i32))?;
    let unknown7 = r.blob(29)?;
    let mut local_names = r.vec_capacity(100, 5)?;
    for _ in 0..100 {
        local_names.push(r.string()?);
    }
    r.expect(&[0x91])?;
    let unknown9 = r.string()?;
    let tail_marker = r.u8()?;
    let tail = match tail_marker {
        0x92 => {
            let s = r.string()?;
            let n = r.u32()?;
            r.expect(&[0x92])?;
            Some((s, n))
        }
        0x91 => None,
        _ => return Err(r.error("unknown common-event tail marker")),
    };
    Ok(CommonEvent {
        span: r.span(start),
        id,
        unknown1,
        unknown2,
        name,
        commands,
        metadata: CommonMetadata {
            unknown11,
            description,
            unknown3,
            unknown4,
            unknown5,
            unknown6,
            unknown7,
            local_names,
            unknown9,
            tail_marker,
            tail,
        },
    })
}
pub fn parse_common(input: &[u8], limits: &Limits) -> Result<Parsed<CommonEvents>> {
    let mut header = Reader::new(input, 0, Domain::Stored, limits)?;
    header.expect(&[0, 0x57, 0, 0, 0x4f, 0x4c, 0x55, 0x46, 0x43, 0, 0xcc])?;
    let reconstructed = lz4::reconstruct(input, 11, limits)?;
    let mut r = Reader::new(&reconstructed, 11, Domain::Reconstructed, limits)?;
    let events = r.vector(1, common_event)?;
    let terminator = r.u8()?;
    if terminator != 0x92 {
        return Err(r.error("commonCC profile requires final 0x92"));
    }
    r.eof()?;
    Ok(Parsed {
        value: CommonEvents { events, terminator },
        reconstructed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_extensions_and_route_namespace_survive() {
        let limits = Limits::default();
        let mut bytes = vec![1];
        bytes.extend_from_slice(&201u32.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 1, 3, 3, 3, 0, 3, 2]);
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&[32, 0, 1, 0, 2, 0xaa, 0xbb]);
        bytes.extend_from_slice(&[1, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut r = Reader::new(&bytes, 0, Domain::Reconstructed, &limits).unwrap();
        let c = command(&mut r).unwrap();
        assert_eq!(c.extension, [0xaa, 0xbb]);
        assert_eq!(c.movement.unwrap().routes[0].opcode, 32);
        assert_eq!(command(&mut r).unwrap().opcode, 0);
        r.eof().unwrap();
        for n in 0..c.span.end {
            assert!(command(
                &mut Reader::new(&bytes[..n], 0, Domain::Reconstructed, &limits).unwrap()
            )
            .is_err());
        }
    }
    #[test]
    fn oversized_string_and_ast_budget_fail_before_copy() {
        let limits = Limits {
            max_ast_bytes: 2,
            ..Limits::default()
        };
        let mut r = Reader::new(
            &[3, 0, 0, 0, b'a', b'b', 0],
            0,
            Domain::Reconstructed,
            &limits,
        )
        .unwrap();
        assert!(r.string().unwrap_err().message.contains("budget"));
        assert!(Reader::new(&[255; 4], 0, Domain::Reconstructed, &limits)
            .unwrap()
            .string()
            .is_err());
    }
    #[test]
    fn map100_has_no_compression_header_and_rejects_unverified_events() {
        let mut bytes = vec![0; 10];
        bytes.extend_from_slice(b"WOLFM\0");
        bytes.extend_from_slice(&[0x55, 0, 0, 0, 100, 0, 0, 0, 0x66]);
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.push(0); // Empty NUL-terminated label.
        for word in [0u32, 20, 15, 0, u32::MAX] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.push(0x66);
        let limits = Limits::default();
        let map = parse_map(&bytes, &limits).unwrap();
        assert_eq!(map.reconstructed, bytes);
        assert_eq!(map.value.unknown, None);
        assert_eq!(map.value.tiles, None);
        assert_eq!(map.value.layers, 3);
        for n in 0..bytes.len() {
            assert!(parse_map(&bytes[..n], &limits).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(parse_map(&trailing, &limits)
            .unwrap_err()
            .message
            .contains("trailing"));
        bytes[42] = 1;
        assert!(parse_map(&bytes, &limits)
            .unwrap_err()
            .message
            .contains("events"));
        bytes[20] = 102;
        assert!(parse_map(&bytes, &limits)
            .unwrap_err()
            .message
            .contains("version"));
    }
}
