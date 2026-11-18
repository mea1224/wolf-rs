//! Observed compressed C4 database layout; spans exclude the optional notice.
use crate::{lz4, reader::Reader, Domain, Limits, Parsed, Result, Span, WolfString};

#[derive(Debug)]
pub struct Database {
    pub types: Vec<DatabaseType>,
}
#[derive(Debug)]
pub struct DatabaseType {
    pub span: Span,
    pub tag: u32,
    pub extra: Option<WolfString>,
    pub fields: Vec<u32>,
    pub rows: Vec<DatabaseRow>,
}
#[derive(Debug)]
pub struct DatabaseRow {
    pub span: Span,
    pub integers: Vec<i32>,
    pub strings: Vec<WolfString>,
}
impl DatabaseType {
    pub fn integer(&self, row: usize, field: usize) -> Option<i32> {
        let slot = self.fields.get(field)?.checked_sub(1000)?;
        if slot >= 1000 {
            return None;
        }
        self.rows.get(row)?.integers.get(slot as usize).copied()
    }
    pub fn string(&self, row: usize, field: usize) -> Option<&WolfString> {
        let slot = self.fields.get(field)?.checked_sub(2000)?;
        self.rows.get(row)?.strings.get(slot as usize)
    }
}

pub fn parse_database(input: &[u8], limits: &Limits) -> Result<Parsed<Database>> {
    let mut stored = Reader::new(input, 0, Domain::Stored, limits)?;
    if input.len() > limits.max_stored_bytes {
        return Err(stored.error("stored file size limit"));
    }
    let input = input
        .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
        .unwrap_or(input);
    stored = Reader::new(input, 0, Domain::Stored, limits)?;
    stored.expect(&[0, 0x57, 0, 0, 0x4f, 0x4c, 0x55, 0x46, 0x4d, 0, 0xc4])?;
    let reconstructed = lz4::reconstruct(input, 11, limits)?;
    let mut r = Reader::new(&reconstructed, 11, Domain::Reconstructed, limits)?;
    let types = r.vector(16, |r| {
        let start = r.pos;
        r.expect(&0xffff_fffe_u32.to_le_bytes())?;
        let tag = r.u32()?;
        let count = r.count()?;
        let extra = if tag == 120000 {
            Some(r.string()?)
        } else {
            None
        };
        let mut fields = r.vec_capacity(count, 4)?;
        for _ in 0..count {
            fields.push(r.u32()?);
        }
        let integers = fields
            .iter()
            .filter(|&&f| (1000..2000).contains(&f))
            .count();
        let strings = count - integers;
        for &field in &fields {
            if !((1000..2000).contains(&field) && (field - 1000) < integers as u32
                || field >= 2000 && (field - 2000) < strings as u32)
            {
                return Err(r.error("unsupported database field encoding"));
            }
        }
        let rows = r.vector(
            count
                .checked_mul(4)
                .ok_or_else(|| r.error("row size overflow"))?,
            |r| {
                let start = r.pos;
                let mut int_values = r.vec_capacity(integers, 4)?;
                for _ in 0..integers {
                    int_values.push(r.i32()?);
                }
                let mut str_values = r.vec_capacity(strings, 5)?;
                for _ in 0..strings {
                    str_values.push(r.string()?);
                }
                Ok(DatabaseRow {
                    span: r.span(start),
                    integers: int_values,
                    strings: str_values,
                })
            },
        )?;
        Ok(DatabaseType {
            span: r.span(start),
            tag,
            extra,
            fields,
            rows,
        })
    })?;
    r.expect(&[0xc4])?;
    r.eof()?;
    Ok(Parsed {
        value: Database { types },
        reconstructed,
    })
}
