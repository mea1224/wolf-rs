//! Observed map100/map103/common-CC profiles. Raw syntax, no gameplay semantics.
mod database;
mod game;
mod lz4;
mod parse;
mod reader;
mod tileset;

pub use database::{parse_database, Database, DatabaseRow, DatabaseType};
pub use game::{parse_game, GameConfig};
pub use tileset::{parse_tilesets, TileSet};

pub use lz4::decode_block;
pub use parse::{parse_common, parse_map};
use serde::{Deserialize, Serialize};

pub const PROFILE: &str = "wolf-loose-map100-map103-commonCC-v1";

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_stored_bytes: usize,
    pub max_decoded_bytes: usize,
    pub max_ast_bytes: usize,
    pub max_string_bytes: usize,
    pub max_records: usize,
    pub max_tile_words: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_stored_bytes: 128 << 20,
            max_decoded_bytes: 128 << 20,
            max_ast_bytes: 256 << 20,
            max_string_bytes: 4 << 20,
            max_records: 1_000_000,
            max_tile_words: 16_000_000,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Domain {
    Stored,
    Reconstructed,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub offset: usize,
    pub domain: Domain,
    pub message: String,
}
impl Error {
    pub(crate) fn new(offset: usize, domain: Domain, message: impl Into<String>) -> Self {
        Self {
            offset,
            domain,
            message: message.into(),
        }
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:?} offset 0x{:x}: {}",
            self.domain, self.offset, self.message
        )
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

/// Every AST span is in the prefix-preserving reconstructed stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WolfString {
    /// Includes the stored terminal NUL; the u32 length lives in `span`.
    pub raw: Vec<u8>,
    pub span: Span,
}
impl WolfString {
    pub fn text(&self) -> std::result::Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self.raw.strip_suffix(&[0]).unwrap_or(&self.raw))
    }
}
#[derive(Debug)]
pub struct Parsed<T> {
    pub value: T,
    pub reconstructed: Vec<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteCommand {
    pub span: Span,
    pub opcode: u8,
    pub args: Vec<i32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveData {
    pub unknown: Vec<u8>,
    pub flags: u8,
    pub routes: Vec<RouteCommand>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCommand {
    pub span: Span,
    pub opcode: u32,
    pub args: Vec<i32>,
    pub indent: u8,
    pub strings: Vec<WolfString>,
    pub terminator: u8,
    pub movement: Option<MoveData>,
    pub extension_length: u8,
    pub extension: Vec<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub span: Span,
    pub unknown: u32,
    pub graphic: WolfString,
    pub graphic_fields: Vec<u8>,
    pub conditions: Vec<u8>,
    pub movement: Vec<u8>,
    pub flags: u8,
    pub route_flags: u8,
    pub routes: Vec<RouteCommand>,
    pub commands: Vec<RawCommand>,
    pub features: u32,
    pub shadow_collision: Vec<u8>,
    pub page_transfer: Option<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapEvent {
    pub span: Span,
    pub id: u32,
    pub name: WolfString,
    pub x: u32,
    pub y: u32,
    pub pages: Vec<Page>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Map {
    pub version: u32,
    pub unknown_byte_24: u8,
    pub label: WolfString,
    pub tileset: u32,
    pub width: u32,
    pub height: u32,
    pub unknown: Option<u32>,
    pub layers: u32,
    /// Contiguous signed words in observed storage order; x/y meaning not assumed.
    pub tiles: Option<Vec<i32>>,
    pub tiles_span: Span,
    pub events: Vec<MapEvent>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommonMetadata {
    pub unknown11: WolfString,
    pub description: WolfString,
    pub unknown3: Vec<WolfString>,
    pub unknown4: Vec<u8>,
    pub unknown5: Vec<Vec<WolfString>>,
    pub unknown6: Vec<Vec<i32>>,
    pub unknown7: Vec<u8>,
    pub local_names: Vec<WolfString>,
    pub unknown9: WolfString,
    pub tail_marker: u8,
    pub tail: Option<(WolfString, u32)>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommonEvent {
    pub span: Span,
    pub id: u32,
    pub unknown1: u32,
    pub unknown2: Vec<u8>,
    pub name: WolfString,
    pub commands: Vec<RawCommand>,
    pub metadata: CommonMetadata,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommonEvents {
    pub events: Vec<CommonEvent>,
    pub terminator: u8,
}
