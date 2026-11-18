//! Native database operands and the plain-text subset of 94FC50 expansion.
//! Evidence: validation/native-values/{database-read,database-load,text-tokenize,text-expand}.asm.
//! UI formatting controls and malformed-token recovery are deliberately
//! rejected; those require distinct semantics, not a successful empty placeholder.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wolf_format::Database;

/// System string 60: 9EB45C..9EB644 replaces these tokens in this order,
/// then stores the resulting title. `software_label` is empty in hardware mode;
/// the platform supplies the original localized label when software mode is active.
pub fn expand_window_title(
    template: &str,
    game_title: &str,
    title_plus: &str,
    software_label: &str,
) -> Result<String, String> {
    const LIMIT: usize = 16 * 1024 * 1024;
    if [template, game_title, title_plus, software_label]
        .iter()
        .any(|s| s.len() > LIMIT || s.contains('\0'))
    {
        return Err("native window title byte limit or embedded NUL".into());
    }
    let mut text = template.to_owned();
    // 4259D0 resumes after the inserted text, so replacements are not recursive
    // within a pass. A later token can still expand inside an earlier replacement.
    for (token, value) in [
        ("<TITLE>", game_title),
        ("<TITLE_PLUS>", title_plus),
        ("<SOFT>", software_label),
    ] {
        let count = text.matches(token).count();
        let kept = text.len() - count * token.len();
        if count
            .checked_mul(value.len())
            .and_then(|n| kept.checked_add(n))
            .is_none_or(|n| n > LIMIT)
        {
            return Err("native expanded window title byte limit".into());
        }
        text = text.replace(token, value);
    }
    Ok(text)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DatabaseKind {
    User,
    Mutable,
    System,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseAddress {
    pub kind: DatabaseKind,
    pub type_id: usize,
    pub row: usize,
    pub field: usize,
}
impl DatabaseAddress {
    /// The decimal split is from 9E51BE..9E5383, not inferred from game labels.
    pub fn decode(encoded: i32) -> Option<Self> {
        let (kind, base) = match encoded {
            1_000_000_000..=1_099_999_999 => (DatabaseKind::User, 1_000_000_000),
            1_100_000_000..=1_199_999_999 => (DatabaseKind::Mutable, 1_100_000_000),
            1_300_000_000..=1_399_999_999 => (DatabaseKind::System, 1_300_000_000),
            _ => return None,
        };
        Some(Self {
            kind,
            type_id: ((encoded - base) / 1_000_000) as usize,
            row: ((encoded % 1_000_000) / 100) as usize,
            field: (encoded % 100) as usize,
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NativeValue {
    Number(i32),
    Text(String),
}
impl NativeValue {
    pub fn text(self) -> String {
        match self {
            Self::Number(n) => n.to_string(),
            Self::Text(s) => s,
        }
    }
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct NativeDatabases {
    tables: BTreeMap<DatabaseKind, Vec<Vec<Vec<NativeValue>>>>,
    field_counts: BTreeMap<DatabaseKind, Vec<usize>>,
    numeric_fields: BTreeMap<DatabaseKind, Vec<Vec<bool>>>,
    names: BTreeMap<DatabaseKind, Vec<DatabaseNames>>,
    row_modes: BTreeMap<DatabaseKind, Vec<RowNameMode>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RowNameMode {
    tag: u32,
    reference_name: Option<String>,
    first_string_field: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DatabaseNames {
    name: String,
    fields: Vec<String>,
    rows: Vec<String>,
    numeric_defaults: Vec<i32>,
}

/// A prepared raw-250 effect. Variable writes belong to the VM, database writes to this store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseEffect {
    Reset { type_id: usize, row: Option<usize> },
    Read {
        destination: i32,
        value: NativeValue,
        operator: u8,
    },
    Write {
        address: DatabaseAddress,
        value: NativeValue,
        operator: u8,
    },
}

/// Raw 250 uses x86 wrapping arithmetic and substitutes 1 for zero divisors.
/// Mixed string/numeric coercion depends on an unverified engine setting and is rejected.
pub fn assign_native_value(
    old: NativeValue,
    source: NativeValue,
    operator: u8,
) -> Result<NativeValue, String> {
    match (old, source) {
        (NativeValue::Number(a), NativeValue::Number(b)) => {
            let divisor = if b == 0 { 1 } else { b };
            Ok(NativeValue::Number(match operator {
                0 => b,
                1 => a.wrapping_add(b),
                2 => a.wrapping_sub(b),
                3 => a.wrapping_mul(b),
                4 => a.checked_div(divisor).ok_or("native division overflow")?,
                5 => a.checked_rem(divisor).ok_or("native remainder overflow")?,
                6 => a.max(b),
                7 => a.min(b),
                _ => return Err("unsupported database assignment operator".into()),
            }))
        }
        (NativeValue::Text(mut a), NativeValue::Text(b)) => match operator {
            0 => Ok(NativeValue::Text(b)),
            1 => {
                if a.len().saturating_add(b.len()) > 16 * 1024 * 1024 {
                    return Err("database string size limit".into());
                }
                a.push_str(&b);
                Ok(NativeValue::Text(a))
            }
            _ => Err("unsupported database string assignment operator".into()),
        },
        _ => Err("database assignment changes field type".into()),
    }
}
impl NativeDatabases {
    pub fn insert(&mut self, kind: DatabaseKind, source: &Database) -> Result<(), String> {
        if self.names.get(&kind).is_some_and(|names| {
            names.len() != source.types.len()
                || names.iter().zip(&source.types).any(|(name, ty)| {
                    name.fields.len() != ty.fields.len()
                        || name.numeric_defaults.len() != ty.fields.len()
                })
        }) {
            return Err("database project/schema count mismatch".into());
        }
        let mut types = Vec::with_capacity(source.types.len());
        for (type_id, ty) in source.types.iter().enumerate() {
            let mut rows = Vec::with_capacity(ty.rows.len());
            for row in 0..ty.rows.len() {
                let mut fields = Vec::with_capacity(ty.fields.len());
                for field in 0..ty.fields.len() {
                    if let Some(n) = ty.integer(row, field) {
                        fields.push(NativeValue::Number(n));
                    } else if let Some(s) = ty.string(row, field) {
                        fields.push(NativeValue::Text(
                            s.text().map_err(|e| e.to_string())?.into(),
                        ));
                    } else {
                        return Err(format!(
                            "invalid database {kind:?} type {type_id} row {row} field {field}"
                        ));
                    }
                }
                rows.push(fields);
            }
            types.push(rows);
        }
        self.tables.insert(kind, types);
        self.field_counts
            .insert(kind, source.types.iter().map(|t| t.fields.len()).collect());
        self.numeric_fields.insert(
            kind,
            source
                .types
                .iter()
                .map(|t| t.fields.iter().map(|f| (1000..2000).contains(f)).collect())
                .collect(),
        );
        let modes = source
            .types
            .iter()
            .map(|ty| {
                Ok(RowNameMode {
                    tag: ty.tag,
                    reference_name: ty
                        .extra
                        .as_ref()
                        .map(|s| s.text().map(str::to_owned).map_err(|e| e.to_string()))
                        .transpose()?,
                    first_string_field: ty.fields.iter().position(|&f| f == 2000),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.row_modes.insert(kind, modes);
        Ok(())
    }

    /// 947C80 resolves a row name dynamically, including cross-bank references.
    /// 964BBA..964C00 binds reference group 1=System, 2=User, 3=Mutable in every bank.
    pub fn row_name(
        &self,
        kind: DatabaseKind,
        type_id: usize,
        row: usize,
    ) -> Result<String, String> {
        self.row_name_at_depth(kind, type_id, row, 0)
    }

    fn row_name_at_depth(
        &self,
        kind: DatabaseKind,
        type_id: usize,
        row: usize,
        depth: u32,
    ) -> Result<String, String> {
        if depth > 50 {
            return Err("database row-name recursion limit".into());
        }
        let mode = self
            .row_modes
            .get(&kind)
            .and_then(|v| v.get(type_id))
            .ok_or("missing database row-name mode")?;
        self.tables
            .get(&kind)
            .and_then(|t| t.get(type_id))
            .and_then(|t| t.get(row))
            .ok_or("missing database row-name row")?;
        match mode.tag {
            0 => self
                .names
                .get(&kind)
                .and_then(|t| t.get(type_id))
                .and_then(|t| t.rows.get(row))
                .cloned()
                .ok_or_else(|| "missing database project row name".into()),
            1 => {
                if let Some(field) = mode.first_string_field {
                    match self.get(DatabaseAddress {
                        kind,
                        type_id,
                        row,
                        field,
                    })? {
                        NativeValue::Text(s) => Ok(s.clone()),
                        _ => Err("row name refers to numeric field".into()),
                    }
                } else {
                    Ok(String::new())
                }
            }
            2 => {
                let modes = self.row_modes.get(&kind).ok_or("missing database modes")?;
                let previous = (0..type_id)
                    .rev()
                    .find(|&id| modes[id].tag != 2)
                    .ok_or("row-name previous type unavailable")?;
                self.row_name_at_depth(kind, previous, row, depth + 1)
            }
            tag => {
                let (group, id, by_name) = if (100_000..200_000).contains(&tag) {
                    ((tag % 100_000) / 10_000, 0, true)
                } else {
                    (tag / 10_000, (tag % 10_000) as usize, false)
                };
                let referenced = match group {
                    1 => DatabaseKind::System,
                    2 => DatabaseKind::User,
                    3 => DatabaseKind::Mutable,
                    _ => return Err(format!("unsupported database row-name mode {tag}")),
                };
                let id = if by_name {
                    let name = mode
                        .reference_name
                        .as_ref()
                        .ok_or("missing row-name reference type name")?;
                    unique_name(
                        self.names
                            .get(&referenced)
                            .ok_or("missing referenced project")?
                            .iter()
                            .map(|t| t.name.as_str()),
                        name,
                    )?
                } else {
                    id
                };
                let target = self
                    .row_modes
                    .get(&referenced)
                    .and_then(|v| v.get(id))
                    .ok_or("missing referenced row-name type")?;
                if target.tag == mode.tag {
                    return Err("self-referencing database row-name mode".into());
                }
                self.row_name_at_depth(referenced, id, row, depth + 1)
            }
        }
    }
    /// Parse the UTF-8 project metadata layout verified against all three shipped projects.
    /// Names and numeric defaults are retained; unrelated editor metadata is validated and skipped.
    pub fn insert_project(&mut self, kind: DatabaseKind, bytes: &[u8]) -> Result<(), String> {
        let mut reader = ProjectReader {
            bytes,
            pos: 0,
            allocated: 0,
        };
        if bytes.len() > 32 * 1024 * 1024 {
            return Err("database project size limit".into());
        }
        let count = reader.count(4)?;
        let mut names = Vec::new();
        for _ in 0..count {
            let name = reader.string()?;
            let fields = reader.strings()?;
            let rows = reader.strings()?;
            reader.string()?; // description
            let count = reader.count(1)?;
            reader.take(count)?; // field editor settings
            reader.strings()?;
            let count = reader.count(4)?;
            for _ in 0..count {
                reader.strings()?;
            }
            let count = reader.count(4)?;
            for _ in 0..count {
                let n = reader.count(4)?;
                reader.take(n * 4)?;
            }
            let count = reader.count(4)?;
            if count != fields.len() {
                return Err("database project default/field count mismatch".into());
            }
            let numeric_defaults = (0..count)
                .map(|_| {
                    reader
                        .take(4)
                        .map(|b| i32::from_le_bytes(b.try_into().unwrap()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            names.push(DatabaseNames {
                name,
                fields,
                rows,
                numeric_defaults,
            });
        }
        if reader.pos != bytes.len() {
            return Err("database project trailing bytes".into());
        }
        if let Some(counts) = self.field_counts.get(&kind) {
            if names.len() != counts.len()
                || names
                    .iter()
                    .zip(counts)
                    .any(|(n, count)| n.fields.len() != *count)
            {
                return Err("database project/schema count mismatch".into());
            }
        }
        self.names.insert(kind, names);
        Ok(())
    }

    /// Decode the ordinary read/write forms of A1FF30 and field -3 row-name/index reads.
    /// Other metadata selectors and mode families remain errors. `read` resolves an original operand,
    /// including literal integers; `expand` applies 94FC50 in the current event context.
    pub fn prepare_command(
        &self,
        args: &[i32],
        strings: &[String],
        mut read: impl FnMut(i32) -> Result<NativeValue, String>,
        mut expand: impl FnMut(&str) -> Result<String, String>,
    ) -> Result<DatabaseEffect, String> {
        if args.len() != 5 {
            return Err("database command requires five arguments".into());
        }
        let flags = args[3] as u32;
        if flags & !0x7_FFFF != 0 {
            return Err("unsupported database command flag bits".into());
        }
        let kind = match flags & 0xF00 {
            0 => DatabaseKind::Mutable,
            0x100 => DatabaseKind::System,
            0x200 => DatabaseKind::User,
            _ => return Err("unsupported database bank".into()),
        };
        let mode = flags & 0xF000;
        if mode != 0 && mode != 0x1000 {
            return Err("unsupported database command mode".into());
        }
        let source_mode = flags & 0xF;
        if source_mode > 2 {
            return Err("unsupported database source mode".into());
        }
        let operator = ((flags >> 4) & 0xF) as u8;
        if operator > 7 {
            return Err("unsupported database assignment operator".into());
        }
        let mut selectors = Vec::new();
        for (i, argument) in args[..3].iter().enumerate() {
            selectors.push(if flags & (0x10000 << i) != 0 {
                NativeValue::Text(expand(
                    strings
                        .get(i + 1)
                        .ok_or("missing named database selector")?,
                )?)
            } else {
                read(*argument)?
            });
        }
        let names = self.names.get(&kind);
        let type_id = match &selectors[0] {
            NativeValue::Number(n) => {
                usize::try_from(*n).map_err(|_| "unsupported negative database type")?
            }
            NativeValue::Text(name) => unique_name(
                names
                    .ok_or("missing database project names")?
                    .iter()
                    .map(|n| n.name.as_str()),
                name,
            )?,
        };
        let ty = self
            .tables
            .get(&kind)
            .and_then(|t| t.get(type_id))
            .ok_or("missing database type")?;
        let project = names.and_then(|n| n.get(type_id));
        // A206A8 checks raw -2; A226A9/A228DE resolve it before dispatch.
        // Whole-type reset restores the project's row count, then 944D50
        // initializes each row to numeric project defaults and empty strings.
        if mode == 0 && (args[1] == -2 || args[2] == -2) {
            if kind != DatabaseKind::Mutable { return Err("reset requires mutable database".into()); }
            let row = if args[1] == -2 { None } else {
                Some(selector_index(&selectors[1], project.map(|p| p.rows.as_slice()))?)
            };
            if row.is_some_and(|row| row >= ty.len()) { return Err("reset row out of bounds".into()); }
            return Ok(DatabaseEffect::Reset { type_id, row });
        }
        // Raw field -3 is dispatched before count queries (A232EC vs A24570).
        // Combined row/field -3 is a distinct type-name query and remains unsupported.
        let row_name_query = mode == 0x1000 && args[2] == -3 && args[1] != -3;
        let count_query = mode == 0x1000 && !row_name_query && (args[1] == -1 || args[2] == -1);
        let row = if count_query {
            0
        } else {
            match &selectors[1] {
                NativeValue::Number(n) => {
                    usize::try_from(*n).map_err(|_| "unsupported negative database row")?
                }
                NativeValue::Text(name) => {
                    // 965070 inserts in descending row order; the smallest duplicate index wins.
                    let mut found = None;
                    for row in 0..ty.len() {
                        if self.row_name(kind, type_id, row)? == *name {
                            found = Some(row);
                            break;
                        }
                    }
                    found.ok_or_else(|| format!("database row name not found: {name:?}"))?
                }
            }
        };
        let field = if count_query || row_name_query {
            0
        } else {
            selector_index(&selectors[2], project.map(|p| p.fields.as_slice()))?
        };
        let address = DatabaseAddress {
            kind,
            type_id,
            row,
            field,
        };
        if mode == 0x1000 {
            if source_mode == 2 {
                return Err(
                    "literal-string mode is invalid for a database read destination".into(),
                );
            }
            let destination = if source_mode == 1 {
                match read(args[4])? {
                    NativeValue::Number(n) => n,
                    _ => return Err("database indirect destination is not numeric".into()),
                }
            } else {
                args[4]
            };
            let value = if row_name_query {
                match &selectors[1] {
                    // A234C3 -> 947C80, then 94FC50 and text set/append.
                    NativeValue::Number(_) => {
                        NativeValue::Text(expand(&self.row_name(kind, type_id, row)?)?)
                    }
                    // A2380F..A238BE returns the already resolved row index when
                    // the row operand was a name (including the explicit name flag).
                    NativeValue::Text(_) => NativeValue::Number(
                        i32::try_from(row).map_err(|_| "database row index overflow")?,
                    ),
                }
            } else if count_query {
                let count = if args[2] == -1 {
                    *self
                        .field_counts
                        .get(&kind)
                        .and_then(|v| v.get(type_id))
                        .ok_or("missing database field count")?
                } else if kind == DatabaseKind::System && (1..=3).contains(&type_id) {
                    ty.len()
                        .checked_sub(10)
                        .ok_or("system database count smaller than reserved rows")?
                } else {
                    ty.len()
                };
                NativeValue::Number(i32::try_from(count).map_err(|_| "database count overflow")?)
            } else {
                match self.read_value(address)? {
                    NativeValue::Number(n) => NativeValue::Number(n),
                    NativeValue::Text(s) => NativeValue::Text(expand(&s)?),
                }
            };
            Ok(DatabaseEffect::Read {
                destination,
                value,
                operator,
            })
        } else {
            if kind != DatabaseKind::Mutable {
                return Err("only mutable database writes are supported".into());
            }
            self.value_before_write(address)?;
            let value = match source_mode {
                0 => read(args[4])?,
                1 => match read(args[4])? {
                    NativeValue::Number(n) => read(n)?,
                    _ => return Err("database indirect source is not numeric".into()),
                },
                2 => NativeValue::Text(expand(
                    strings.first().ok_or("missing database string source")?,
                )?),
                _ => unreachable!(),
            };
            Ok(DatabaseEffect::Write {
                address,
                value,
                operator,
            })
        }
    }

    pub fn apply_write(&mut self, effect: DatabaseEffect) -> Result<(), String> {
        match effect {
            DatabaseEffect::Reset { type_id, row } => {
                let kind = DatabaseKind::Mutable;
                let project = self.names.get(&kind).and_then(|types| types.get(type_id)).ok_or("reset needs project schema")?;
                let fields = &self.numeric_fields[&kind][type_id];
                let initial: Vec<_> = fields.iter().enumerate().map(|(i, numeric)| if *numeric {
                    NativeValue::Number(project.numeric_defaults[i])
                } else { NativeValue::Text(String::new()) }).collect();
                let rows = &mut self.tables.get_mut(&kind).ok_or("missing mutable database")?[type_id];
                if let Some(row) = row {
                    *rows.get_mut(row).ok_or("reset row out of bounds")? = initial;
                } else {
                    if project.rows.len() > 10_000 || project.rows.len().saturating_mul(fields.len()) > 1_000_000 { return Err("database reset allocation limit".into()); }
                    *rows = vec![initial; project.rows.len()];
                }
                Ok(())
            }
            DatabaseEffect::Write {
                address,
                value,
                operator,
            } => {
                let value =
                    assign_native_value(self.value_before_write(address)?, value, operator)?;
                self.write_mutable(address, value)
            }
            DatabaseEffect::Read { .. } => {
                Err("database read result must be applied to VM variables".into())
            }
        }
    }
    pub fn get(&self, at: DatabaseAddress) -> Result<&NativeValue, String> {
        self.tables
            .get(&at.kind)
            .and_then(|t| t.get(at.type_id))
            .and_then(|t| t.get(at.row))
            .and_then(|r| r.get(at.field))
            .ok_or_else(|| format!("missing native database cell {at:?}"))
    }
    pub fn number(&self, encoded: i32) -> Result<i32, String> {
        let at = DatabaseAddress::decode(encoded).ok_or("operand is not a database address")?;
        match self.read_value(at)? {
            NativeValue::Number(n) => Ok(n),
            // 946A62..946A76 returns zero for a present nonnumeric descriptor.
            NativeValue::Text(_) => Ok(0),
        }
    }
    /// 946A54 -> 946ABF -> 946BEB: Mutable numeric reads beyond the row vector
    /// use project[type].defaults[field]. Writes and string reads remain strict.
    fn read_value(&self, at: DatabaseAddress) -> Result<NativeValue, String> {
        let rows = self
            .tables
            .get(&at.kind)
            .and_then(|types| types.get(at.type_id))
            .ok_or_else(|| format!("missing native database type {at:?}"))?;
        if at.row < rows.len() || at.kind != DatabaseKind::Mutable {
            return self.get(at).cloned();
        }
        let numeric = self
            .numeric_fields
            .get(&at.kind)
            .and_then(|types| types.get(at.type_id))
            .and_then(|fields| fields.get(at.field))
            .ok_or_else(|| format!("missing native database field {at:?}"))?;
        if !numeric {
            return Err(format!("missing native string database row {at:?}"));
        }
        self.names
            .get(&at.kind)
            .and_then(|types| types.get(at.type_id))
            .and_then(|project| project.numeric_defaults.get(at.field))
            .copied()
            .map(NativeValue::Number)
            .ok_or_else(|| format!("missing native database project default {at:?}"))
    }

    /// Validate serialized schema before any restored database operation is executed.
    /// A bank may lack project metadata; its out-of-row reads then remain errors.
    pub fn validate(&self) -> Result<(), String> {
        if self.tables.keys().ne(self.field_counts.keys())
            || self.tables.keys().ne(self.numeric_fields.keys())
            || self.tables.keys().ne(self.row_modes.keys())
            || self
                .names
                .keys()
                .any(|kind| !self.tables.contains_key(kind))
        {
            return Err("saved database schema bank mismatch".into());
        }
        for (kind, types) in &self.tables {
            let counts = &self.field_counts[kind];
            let numeric = &self.numeric_fields[kind];
            let modes = &self.row_modes[kind];
            let project = self.names.get(kind);
            if types.len() != counts.len()
                || types.len() != numeric.len()
                || types.len() != modes.len()
                || project.is_some_and(|p| p.len() != types.len())
            {
                return Err("saved database schema type count mismatch".into());
            }
            for (id, rows) in types.iter().enumerate() {
                let count = counts[id];
                if numeric[id].len() != count
                    || project.is_some_and(|p| {
                        p[id].fields.len() != count || p[id].numeric_defaults.len() != count
                    })
                    || modes[id].first_string_field.is_some() != numeric[id].contains(&false)
                    || modes[id]
                        .first_string_field
                        .is_some_and(|field| field >= count || numeric[id][field])
                    || rows.iter().any(|row| {
                        row.len() != count
                            || row.iter().zip(&numeric[id]).any(|(value, numeric)| {
                                matches!(value, NativeValue::Number(_)) != *numeric
                            })
                    })
                {
                    return Err("saved database field/default schema mismatch".into());
                }
            }
        }
        Ok(())
    }
    pub fn write_mutable(&mut self, at: DatabaseAddress, value: NativeValue) -> Result<(), String> {
        if at.kind != DatabaseKind::Mutable {
            return Err("only mutable database writes are connected".into());
        }
        let old = self.value_before_write(at)?;
        if std::mem::discriminant(&old) != std::mem::discriminant(&value) {
            return Err("database assignment changes field type".into());
        }
        let count = self.tables[&at.kind][at.type_id].len();
        if at.row >= count {
            // 9E2FFA..9E3013 and 9E4149..9E4167 resize numeric/string
            // destinations to row + 1. 945830 initializes numeric defaults
            // by logical field index; its new string storage starts empty.
            let numeric = &self.numeric_fields[&at.kind][at.type_id];
            let defaults = self
                .names
                .get(&at.kind)
                .and_then(|types| types.get(at.type_id));
            let row = numeric
                .iter()
                .enumerate()
                .map(|(field, is_numeric)| {
                    if *is_numeric {
                        NativeValue::Number(
                            defaults
                                .and_then(|p| p.numeric_defaults.get(field))
                                .copied()
                                .unwrap_or(0),
                        )
                    } else {
                        NativeValue::Text(String::new())
                    }
                })
                .collect::<Vec<_>>();
            self.tables.get_mut(&at.kind).unwrap()[at.type_id].resize(at.row + 1, row);
        }
        let old = self
            .tables
            .get_mut(&at.kind)
            .and_then(|t| t.get_mut(at.type_id))
            .and_then(|t| t.get_mut(at.row))
            .and_then(|r| r.get_mut(at.field))
            .ok_or_else(|| format!("missing native database cell {at:?}"))?;
        *old = value;
        Ok(())
    }

    fn value_before_write(&self, at: DatabaseAddress) -> Result<NativeValue, String> {
        if at.kind != DatabaseKind::Mutable {
            return Err("only mutable database writes are connected".into());
        }
        let fields = self
            .numeric_fields
            .get(&at.kind)
            .and_then(|types| types.get(at.type_id))
            .ok_or("missing mutable database type")?;
        let numeric = *fields
            .get(at.field)
            .ok_or("missing mutable database field")?;
        // Host allocation limits; not a claim about the native vector's capacity.
        if at.row >= 10_000
            || (at.row + 1)
                .checked_mul(fields.len())
                .is_none_or(|n| n > 1_000_000)
        {
            return Err("mutable database row allocation limit".into());
        }
        if at.row < self.tables[&at.kind][at.type_id].len() {
            return self.get(at).cloned();
        }
        Ok(if numeric {
            NativeValue::Number(
                self.names
                    .get(&at.kind)
                    .and_then(|types| types.get(at.type_id))
                    .and_then(|p| p.numeric_defaults.get(at.field))
                    .copied()
                    .unwrap_or(0),
            )
        } else {
            NativeValue::Text(String::new())
        })
    }
}

fn unique_name<'a>(names: impl Iterator<Item = &'a str>, name: &str) -> Result<usize, String> {
    let mut found = None;
    for (i, candidate) in names.enumerate() {
        if candidate == name {
            if found.is_some() {
                return Err(format!("ambiguous database name {name:?}"));
            }
            found = Some(i);
        }
    }
    found.ok_or_else(|| format!("database name not found: {name:?}"))
}

fn selector_index(value: &NativeValue, names: Option<&[String]>) -> Result<usize, String> {
    match value {
        NativeValue::Number(n) => {
            usize::try_from(*n).map_err(|_| "unsupported negative database selector".into())
        }
        NativeValue::Text(name) => unique_name(
            names
                .ok_or("missing database project names")?
                .iter()
                .map(String::as_str),
            name,
        ),
    }
}

struct ProjectReader<'a> {
    bytes: &'a [u8],
    pos: usize,
    allocated: usize,
}
impl<'a> ProjectReader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(count)
            .ok_or("project offset overflow")?;
        let bytes = self
            .bytes
            .get(self.pos..end)
            .ok_or("truncated database project")?;
        self.pos = end;
        Ok(bytes)
    }
    fn u32(&mut self) -> Result<usize, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()) as usize)
    }
    fn count(&mut self, min_bytes: usize) -> Result<usize, String> {
        let n = self.u32()?;
        if n > 1_000_000 || n > (self.bytes.len() - self.pos) / min_bytes {
            return Err("database project count exceeds bounds".into());
        }
        self.allocated = self
            .allocated
            .checked_add(n.saturating_mul(32))
            .ok_or("project allocation overflow")?;
        if self.allocated > 128 * 1024 * 1024 {
            return Err("database project allocation budget".into());
        }
        Ok(n)
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.u32()?;
        if n == 0 || n > 16 * 1024 * 1024 {
            return Err("database project string length limit".into());
        }
        let raw = self.take(n)?;
        if raw.last() != Some(&0) {
            return Err("unterminated database project string".into());
        }
        std::str::from_utf8(&raw[..n - 1])
            .map(str::to_owned)
            .map_err(|_| "database project string is not UTF-8".into())
    }
    fn strings(&mut self) -> Result<Vec<String>, String> {
        let count = self.count(4)?;
        (0..count).map(|_| self.string()).collect()
    }
}

/// Expand plain asset/config text. `read` receives an absolute original variable address:
/// cself -> 15_000_000 + common*100 + slot; v -> 2_000_000+n; s -> 3_000_000+n;
/// sysS -> 9_900_000+n (token 41, 9512AA..9512BC).
/// The caller must use its current common context; supported display controls are retained.
/// A replacement containing another supported token is expanded in a later pass, like 94FC50.
pub fn expand_plain_text(
    source: &str,
    common: Option<u32>,
    max_bytes: usize,
    read: impl FnMut(i32) -> Result<NativeValue, String>,
) -> Result<String, String> {
    expand_text(source, common, max_bytes, read)
}

/// Expand variable operands while retaining the source spelling of picture controls
/// and path separators. The picture parser, rather than this variable stage, owns
/// their interpretation. Native 94D980 recognizes complete prefixes, not every '\\'.
pub fn expand_native_text(
    source: &str,
    common: Option<u32>,
    max_bytes: usize,
    read: impl FnMut(i32) -> Result<NativeValue, String>,
) -> Result<String, String> {
    expand_text(source, common, max_bytes, read)
}

fn expand_text(
    source: &str,
    common: Option<u32>,
    max_bytes: usize,
    mut read: impl FnMut(i32) -> Result<NativeValue, String>,
) -> Result<String, String> {
    if source.len() > max_bytes {
        return Err("expanded text byte limit".into());
    }
    if source.bytes().any(|c| c == 1) {
        return Err("unsupported text preprocessing/control bytes".into());
    }
    // 94FCC1 gates @ preprocessing; common argument callers pass zero.
    // 94FE82..94FEC3 removes CR bytes; LF remains the native line separator.
    // A0D541..54D and A0D73D..753 also disable @ processing for opcode112.
    let mut result = source.replace('\r', "");
    // Native 954DDE..954DF4 clamps its extra expansion pass count to 100.
    for _ in 0..100 {
        let mut replacements = Vec::new();
        let mut cursor = 0;
        while let Some(offset) = result[cursor..].find('\\') {
            let start = cursor + offset;
            let suffix = &result[start..];
            let (prefix, base, max_index) = if suffix.starts_with("\\cself[") {
                let id = common.ok_or("cself expansion without common context")?;
                let base = i32::try_from(id)
                    .ok()
                    .and_then(|v| v.checked_mul(100))
                    .and_then(|v| v.checked_add(15_000_000))
                    .ok_or("common address overflow")?;
                (7, base, 99)
            } else if suffix.starts_with("\\v[") {
                (3, 2_000_000, 99_999)
            } else if suffix.starts_with("\\s[") {
                (3, 3_000_000, 999_999)
            } else if suffix.starts_with("\\sysS[") {
                (6, 9_900_000, 99_999)
            } else {
                // String assignment A11279..A112BA also passes zero formatting
                // conversion flags. Common10 PC132 appends a literal \\img token.
                // Preserve the recovered picture controls and ordinary Windows
                // path separators; an unimplemented command-looking token must
                // remain diagnosable instead of entering a callee successfully.
                let name_len = suffix[1..]
                    .bytes()
                    .take_while(u8::is_ascii_alphabetic)
                    .count();
                if suffix.as_bytes().get(1 + name_len) == Some(&b'[')
                    && ![
                        "\\f[", "\\mx[", "\\my[", "\\img[", "\\font[", "\\space[", "\\c[", "\\ax[",
                        "\\wE[", "\\cE[", "\\-[",
                    ]
                    .iter()
                    .any(|p| suffix.starts_with(p))
                {
                    return Err(format!("unsupported native text control near {suffix:?}"));
                }
                cursor = start + 1;
                continue;
            };
            let end = suffix[prefix..]
                .find(']')
                .map(|n| start + prefix + n)
                .ok_or("unterminated native text token")?;
            let digits = &result[start + prefix..end];
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Err("nonliteral native text token index".into());
            }
            let index = digits
                .parse::<i32>()
                .map_err(|_| "text token index overflow")?;
            if index > max_index {
                return Err("text token index outside supported range".into());
            }
            replacements.push((start, end + 1, base + index));
            cursor = end + 1;
        }
        if replacements.is_empty() {
            return Ok(result);
        }
        // 94FF3F..94FF50 scans matched token positions in descending order.
        for (start, end, address) in replacements.into_iter().rev() {
            let value = read(address)?.text();
            let length = result
                .len()
                .checked_sub(end - start)
                .and_then(|n| n.checked_add(value.len()))
                .ok_or("expanded text size overflow")?;
            if length > max_bytes {
                return Err("expanded text byte limit".into());
            }
            result.replace_range(start..end, &value);
        }
    }
    let unresolved = ["\\cself[", "\\v[", "\\s[", "\\sysS["]
        .iter()
        .any(|token| result.contains(token));
    if unresolved {
        Err("native text expansion did not settle within 100 passes".into())
    } else {
        Ok(result)
    }
}
