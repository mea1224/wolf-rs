//! A serialization sink that enforces the allocation budget while writing.
pub(crate) struct BoundedBytes {
    pub(crate) bytes: Vec<u8>,
    cap: usize,
}
impl BoundedBytes {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            bytes: Vec::new(),
            cap,
        }
    }
}
impl std::io::Write for BoundedBytes {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if data.len() > self.cap.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("snapshot exceeds byte bound"));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
