//! 122 source 3: A11379..A115FD creates the input session; A06C78..A071DB
//! polls it and writes only on completion. The editor is a portable host UI;
//! its visual placement is not a pixel-exact recreation of the native IME UI.
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use winit::keyboard::{Key, NamedKey};
use wolf_vm::runtime::NativeContext;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TextInput {
    owner: u64,
    program: String,
    pc: usize,
    destination: i32,
    limit: usize,
    can_cancel: bool,
    fallback: String,
    text: String,
    cursor: usize,
    preedit: String,
    position: [i32; 2],
    font: Option<String>,
    complete: Option<bool>,
    #[serde(skip)]
    rendered: Option<(String, Arc<wolf_media::Image>)>,
}

pub(super) fn execute(
    state: &mut Option<TextInput>,
    ctx: &mut NativeContext<'_>,
) -> Result<bool, String> {
    let raw = ctx
        .instruction
        .raw
        .as_ref()
        .ok_or("text input raw record missing")?;
    let a = &raw.args;
    if a.len() != 3 || !raw.strings.is_empty() || a[1] & !0x3013 != 0 || a[1] & 15 != 3 {
        return Err(format!("unsupported native text input form {:?}", a));
    }
    if let Some(input) = state {
        if input.owner != ctx.owner
            || input.program != ctx.frame.program
            || input.pc != ctx.frame.pc
        {
            ctx.retry();
            return Ok(true);
        }
        if let Some(accepted) = input.complete {
            // A0719E writes the saved accumulator on cancel. For assignment
            // that accumulator starts empty, including prefilled input.
            let result = if accepted {
                input.text.clone()
            } else {
                input.fallback.clone()
            };
            ctx.set_text(input.destination, result)?;
            ctx.world
                .systems
                .numbers
                .insert(39, if accepted { 0 } else { -1 });
            *state = None;
            ctx.yield_once();
            return Ok(true);
        }
    } else {
        let destination = if a[1] & 0x10 != 0 {
            ctx.number(a[0])?
        } else {
            a[0]
        };
        let limit = ctx.number(a[2])?.clamp(4, 5000) as usize;
        let text = if a[1] & 0x2000 != 0 {
            ctx.text(destination)?.chars().take(limit).collect()
        } else {
            String::new()
        };
        *state = Some(TextInput {
            owner: ctx.owner,
            program: ctx.frame.program.clone(),
            pc: ctx.frame.pc,
            destination,
            limit,
            can_cancel: a[1] & 0x1000 != 0,
            fallback: String::new(),
            cursor: text.len(),
            text,
            preedit: String::new(),
            position: [40, 41]
                .map(|slot| ctx.world.systems.numbers.get(&slot).copied().unwrap_or(0)),
            font: ctx
                .world
                .systems
                .strings
                .get(&67)
                .filter(|font| !font.is_empty())
                .cloned(),
            complete: None,
            rendered: None,
        });
        ctx.world.systems.numbers.insert(39, 1);
    }
    ctx.retry();
    Ok(true)
}

impl TextInput {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !(4..=5000).contains(&self.limit)
            || self.text.chars().count() > self.limit
            || !self.text.is_char_boundary(self.cursor)
            || self.preedit.len() > 65536
        {
            return Err("invalid saved text input".into());
        }
        Ok(())
    }
    pub(crate) fn commit(&mut self, text: &str) {
        if self.complete.is_some() {
            return;
        }
        let room = self.limit.saturating_sub(self.text.chars().count());
        let insert: String = text
            .chars()
            .filter(|ch| !ch.is_control())
            .take(room)
            .collect();
        self.text.insert_str(self.cursor, &insert);
        self.cursor += insert.len();
        self.preedit.clear();
        self.rendered = None;
    }
    pub(crate) fn preedit(&mut self, text: &str) {
        self.preedit = text.chars().take(self.limit).collect();
        self.rendered = None;
    }
    pub(crate) fn key(&mut self, key: &Key) {
        if self.complete.is_some() || !self.preedit.is_empty() {
            return;
        }
        let previous = self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(at, _)| at);
        let next = self.cursor
            + self.text[self.cursor..]
                .chars()
                .next()
                .map_or(0, char::len_utf8);
        match key {
            Key::Named(NamedKey::Enter) => self.complete = Some(true),
            Key::Named(NamedKey::Escape) if self.can_cancel => self.complete = Some(false),
            Key::Named(NamedKey::ArrowLeft) => self.cursor = previous,
            Key::Named(NamedKey::ArrowRight) => self.cursor = next,
            Key::Named(NamedKey::Home) => self.cursor = 0,
            Key::Named(NamedKey::End) => self.cursor = self.text.len(),
            Key::Named(NamedKey::Backspace) => {
                self.text.replace_range(previous..self.cursor, "");
                self.cursor = previous;
            }
            Key::Named(NamedKey::Delete) => {
                self.text.replace_range(self.cursor..next, "");
            }
            _ => (),
        }
        self.rendered = None;
    }
    pub(crate) fn render(
        &mut self,
        frame: &mut wolf_media::Image,
        font: &str,
    ) -> Result<(), String> {
        let capacity = (frame.dimensions().0 as usize / 28).max(1);
        let before: String = self.text[..self.cursor]
            .chars()
            .rev()
            .take(capacity)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let after: String = self.text[self.cursor..].chars().take(capacity).collect();
        let text = format!(
            "{before}{}|{after}",
            self.preedit.chars().take(capacity).collect::<String>()
        );
        if self.rendered.as_ref().is_none_or(|(old, _)| old != &text) {
            self.rendered = Some((
                text.clone(),
                crate::native_text_win::raster(&text, 28, self.font.as_deref().unwrap_or(font))?
                    .image,
            ));
        }
        let image = &self.rendered.as_ref().unwrap().1;
        let (width, height) = frame.dimensions();
        let (tw, th) = image.dimensions();
        let source = image.bytes();
        let pixels = frame.bytes_mut();
        // Dialog background is authored by the script; native sys40/41 position
        // the input glyphs. Cache rasterization and blend only those glyphs.
        for ty in 0..th {
            for tx in 0..tw {
                let x = self.position[0] + tx as i32;
                let y = self.position[1] + ty as i32;
                if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                    continue;
                }
                let at = ((y as u32 * width + x as u32) * 4) as usize;
                let from = ((ty * tw + tx) * 4) as usize;
                let alpha = u32::from(source[from + 3]);
                for channel in 0..3 {
                    pixels[at + channel] = ((u32::from(source[from + channel]) * alpha
                        + u32::from(pixels[at + channel]) * (255 - alpha))
                        / 255) as u8;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_save_memo_waits_for_unicode_input_and_survives_restore() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let bytes = crate::assets::read_member(&root, "BasicData/CommonEvent.dat").unwrap();
        let bytes = bytes
            .strip_prefix(b"Extracting data from encrypted files violates the guidelines.\0")
            .unwrap_or(&bytes);
        let parsed = wolf_format::parse_common(bytes, &Default::default()).unwrap();
        let event = parsed
            .value
            .events
            .iter()
            .find(|event| event.id == 10)
            .unwrap();
        let raw = event.commands[745].clone();
        assert_eq!(raw.args, [1_600_008, 0x1003, 46]);
        let mut commands = event.commands[735..739].to_vec();
        commands.push(raw);
        let program = wolf_ir::compile(
            "memo".into(),
            "memo".into(),
            wolf_ir::Source {
                asset: "BasicData/CommonEvent.dat".into(),
                content_blake3: "fixture".into(),
                event: 10,
                page: None,
                command: 745,
                span: [0, 0],
            },
            &commands,
        )
        .unwrap();
        let mut vm = wolf_vm::runtime::Vm::new(
            wolf_ir::Library::new(vec![program]).unwrap(),
            wolf_vm::runtime::Config {
                run_until_yield: true,
                ..Default::default()
            },
            1,
            1,
        )
        .unwrap();
        vm.spawn(wolf_ir::Target::Id("memo".into()), 0, vec![])
            .unwrap();
        let mut input = None;
        for _ in 0..3 {
            let result = vm
                .tick_with_host(vec![], &mut |ctx| {
                    if wolf_vm::native_commands::execute_values(ctx)? {
                        return Ok(true);
                    }
                    execute(&mut input, ctx)
                })
                .unwrap();
            assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
            assert_eq!(vm.state().fibers.values().next().unwrap().frames[0].pc, 4);
        }
        input.as_mut().unwrap().commit("保存A");
        assert_eq!(input.as_ref().unwrap().position, [431, 344]);
        assert_eq!(
            input.as_ref().unwrap().font.as_deref(),
            Some("Noto Serif JP")
        );
        assert_eq!(vm.state().world.systems.numbers.get(&39), Some(&1));
        input
            .as_mut()
            .unwrap()
            .key(&Key::Named(NamedKey::Backspace));
        input.as_mut().unwrap().preedit("中");
        input.as_mut().unwrap().key(&Key::Named(NamedKey::Enter));
        assert!(
            input.as_ref().unwrap().complete.is_none(),
            "IME Enter submitted the game input"
        );
        input.as_mut().unwrap().commit("日本語");
        let data = serde_json::to_vec(&input).unwrap();
        input = serde_json::from_slice(&data).unwrap();
        input.as_ref().unwrap().validate().unwrap();
        input.as_mut().unwrap().key(&Key::Named(NamedKey::Enter));
        let mut committed = None;
        let result = vm
            .tick_with_host(vec![], &mut |ctx| {
                let handled = execute(&mut input, ctx)?;
                if input.is_none() {
                    committed = Some(ctx.text(1_600_008)?);
                }
                Ok(handled)
            })
            .unwrap();
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(committed.as_deref(), Some("保存日本語"));
        assert_eq!(vm.state().world.systems.numbers.get(&39), Some(&0));
        assert!(input.is_none());
    }
    #[test]
    fn text_input_cancel_limit_and_unicode_cursor() {
        let mut input = TextInput {
            owner: 0,
            program: "test".into(),
            pc: 0,
            destination: 3_000_000,
            limit: 4,
            can_cancel: false,
            fallback: String::new(),
            text: String::new(),
            cursor: 0,
            preedit: String::new(),
            position: [0, 0],
            font: None,
            complete: None,
            rendered: None,
        };
        input.commit("一二三四五");
        assert_eq!(input.text, "一二三四");
        input.key(&Key::Named(NamedKey::ArrowLeft));
        input.key(&Key::Named(NamedKey::Delete));
        assert_eq!(input.text, "一二三");
        input.key(&Key::Named(NamedKey::Escape));
        assert!(input.complete.is_none());
        input.can_cancel = true;
        input.key(&Key::Named(NamedKey::Escape));
        assert_eq!(input.complete, Some(false));
    }
}
