//! Read current archive SystemDB type11 transition resources; no game execution.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let archive = wolf_assets::wolf350::Archive::open(
        std::fs::read(root.join("Data/BasicData.wolf"))?,
        wolf_assets::wolf350::Limits::default(),
    )?;
    let source = archive.read("SysDatabase.dat")?;
    let data = wolf_format::parse_database(&source.bytes, &wolf_format::Limits::default())?.value;
    let group = &data.types[11];
    for row in 0..group.rows.len() {
        let fields: Vec<_> = (0..group.fields.len()).map(|column| {
            if let Some(s) = group.string(row, column) {
                serde_json::json!({"column":column,"text":s.text().unwrap_or("<invalid encoding>")})
            } else { serde_json::json!({"column":column,"integer":group.integer(row,column)}) }
        }).collect();
        println!(
            "{}",
            serde_json::json!({"bank":"System","type":11,"row":row,"fields":fields})
        );
    }
    Ok(())
}
