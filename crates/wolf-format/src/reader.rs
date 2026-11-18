use crate::{Domain, Error, Limits, Result, Span, WolfString};

pub(crate) struct Reader<'a> {
    pub bytes: &'a [u8],
    pub pos: usize,
    pub domain: Domain,
    pub limits: &'a Limits,
    owned: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8], pos: usize, domain: Domain, limits: &'a Limits) -> Result<Self> {
        if pos > bytes.len() {
            return Err(Error::new(pos, domain, "truncated prefix"));
        }
        Ok(Self {
            bytes,
            pos,
            domain,
            limits,
            owned: 0,
        })
    }
    pub fn error(&self, message: impl Into<String>) -> Error {
        Error::new(self.pos, self.domain, message)
    }
    pub fn span(&self, start: usize) -> Span {
        Span {
            start,
            end: self.pos,
        }
    }
    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| self.error("range overflow"))?;
        let bytes = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| self.error("truncated record"))?;
        self.pos = end;
        Ok(bytes)
    }
    pub fn expect(&mut self, expected: &[u8]) -> Result<()> {
        let start = self.pos;
        if self.take(expected.len())? != expected {
            return Err(Error::new(
                start,
                self.domain,
                format!("expected marker {expected:02x?}"),
            ));
        }
        Ok(())
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn count(&mut self) -> Result<usize> {
        let n =
            usize::try_from(self.u32()?).map_err(|_| self.error("count conversion overflow"))?;
        if n > self.limits.max_records {
            return Err(self.error("record count limit"));
        }
        Ok(n)
    }
    pub fn charge(&mut self, n: usize) -> Result<()> {
        self.owned = self
            .owned
            .checked_add(n)
            .ok_or_else(|| self.error("AST size overflow"))?;
        if self.owned > self.limits.max_ast_bytes {
            return Err(self.error("AST memory budget"));
        }
        Ok(())
    }
    pub fn vec_capacity<T>(&mut self, n: usize, minimum_stored: usize) -> Result<Vec<T>> {
        let min = n
            .checked_mul(minimum_stored)
            .ok_or_else(|| self.error("count overflow"))?;
        if min > self.remaining() {
            return Err(self.error("count exceeds remaining bytes"));
        }
        self.charge(
            n.checked_mul(std::mem::size_of::<T>())
                .ok_or_else(|| self.error("allocation overflow"))?,
        )?;
        let mut out = Vec::new();
        out.try_reserve_exact(n)
            .map_err(|_| self.error("allocation failed"))?;
        Ok(out)
    }
    pub fn vector<T>(
        &mut self,
        min: usize,
        mut parse: impl FnMut(&mut Self) -> Result<T>,
    ) -> Result<Vec<T>> {
        let n = self.count()?;
        let mut out = self.vec_capacity(n, min)?;
        for _ in 0..n {
            out.push(parse(self)?);
        }
        Ok(out)
    }
    pub fn blob(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut out = self.vec_capacity(n, 1)?;
        out.extend_from_slice(self.take(n)?);
        Ok(out)
    }
    pub fn string(&mut self) -> Result<WolfString> {
        let value = self.raw_string()?;
        value.text().map_err(|_| self.error("invalid UTF-8"))?;
        Ok(value)
    }
    pub fn raw_string(&mut self) -> Result<WolfString> {
        let start = self.pos;
        let n = self.u32()? as usize;
        if n == 0 || n > self.limits.max_string_bytes {
            return Err(self.error("string length limit"));
        }
        let raw = self.blob(n)?;
        if raw.last() != Some(&0) {
            return Err(self.error("unterminated string"));
        }
        Ok(WolfString {
            raw,
            span: self.span(start),
        })
    }
    pub fn eof(&self) -> Result<()> {
        if self.remaining() != 0 {
            return Err(self.error("unexpected trailing bytes"));
        }
        Ok(())
    }
}
