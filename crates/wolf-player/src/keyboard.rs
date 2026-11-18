//! Desktop native-index adapter and the engine's permanently accepted fallback keys.
use winit::keyboard::KeyCode;
use wolf_vm::input::KeyboardSample;
#[path = "native_pad.rs"]
mod native_pad;

#[derive(Default)]
pub struct Keyboard {
    sample: KeyboardSample,
    pressed_since_sample: [u64; 4],
    previous_directions: [bool; 4],
    active_direction: Option<usize>,
    active_vertical: Option<usize>,
    active_horizontal: Option<usize>,
}
impl Keyboard {
    pub(crate) fn release_all(&mut self) {
        self.sample.down = [0; 4];
        self.pressed_since_sample = [0; 4];
        self.previous_directions = [false; 4];
        self.active_direction = None;
        self.active_vertical = None;
        self.active_horizontal = None;
    }
    pub fn key(&mut self, key: KeyCode, down: bool) {
        if let Some(index) = index(key) {
            let word = usize::from(index) / 64;
            let bit = 1_u64 << (index % 64);
            if down && self.sample.down[word] & bit == 0 {
                self.pressed_since_sample[word] |= bit;
            }
            self.sample.set(index, down);
        }
    }
    pub fn focused(&mut self, focused: bool) {
        if !focused {
            self.sample.down = [0; 4];
            self.pressed_since_sample = [0; 4];
        }
        self.sample.blocked = !focused;
    }
    #[cfg(test)]
    pub fn sample(&mut self) -> KeyboardSample {
        self.sample_with_policy(wolf_vm::input::InputPolicy::default())
    }
    #[cfg(test)]
    pub fn sample_with_policy(&mut self, policy: wolf_vm::input::InputPolicy) -> KeyboardSample {
        self.sample_with_bindings(policy, ([0; 3], [0; 3]))
    }
    pub fn sample_with_bindings(
        &mut self,
        policy: wolf_vm::input::InputPolicy,
        bindings: ([i32; 3], [i32; 3]),
    ) -> KeyboardSample {
        let (buttons, connected) = native_pad::sample_device();
        let mut sample = self.sample_with_pad_buttons(policy, bindings, buttons);
        sample.pad_connected = connected;
        sample
    }

    /// Supply the device sample before logical mapping; acceptance runs use zero
    /// here so a physical pad cannot leak into configured logical buttons.
    pub fn sample_with_pad_buttons(
        &mut self,
        policy: wolf_vm::input::InputPolicy,
        bindings: ([i32; 3], [i32; 3]),
        pad_buttons: u32,
    ) -> KeyboardSample {
        let mut result = self.sample;
        // Preserve taps received between two ticks. Held keys and Common22's
        // repeat timing still use the normal sampled input path.
        for (down, pressed) in result.down.iter_mut().zip(&mut self.pressed_since_sample) {
            *down |= *pressed;
            *pressed = 0;
        }
        result.pad_buttons = pad_buttons;
        if result.blocked {
            result.pad_buttons = 0;
            self.previous_directions = [false; 4];
            self.active_direction = None;
            self.active_vertical = None;
            self.active_horizontal = None;
            return result;
        }
        let down = |index: usize| result.down[index / 64] & (1 << (index % 64)) != 0;
        // 9BBE31..9BC0xx: cursor arrows and cardinal keypad keys are unconditional
        // fallbacks. Diagonal keypad behavior depends on Game.dat and is not guessed.
        let directions = [
            policy.simulated & 0x80 != 0 || (down(200) || down(72)) && policy.logical & 0x80 != 0,
            policy.simulated & 0x10 != 0 || (down(208) || down(80)) && policy.logical & 0x10 != 0,
            policy.simulated & 0x20 != 0 || (down(203) || down(75)) && policy.logical & 0x20 != 0,
            policy.simulated & 0x40 != 0 || (down(205) || down(77)) && policy.logical & 0x40 != 0,
        ];
        // 9BCDCE..9BCF55: newest direction wins; a release falls back in native order.
        let fallback = [[1, 2, 3], [0, 2, 3], [3, 0, 1], [2, 0, 1]];
        for i in 0..4 {
            if directions[i] && !self.previous_directions[i] {
                self.active_direction = Some(i);
            } else if !directions[i] && self.active_direction == Some(i) {
                self.active_direction = fallback[i].into_iter().find(|j| directions[*j]);
            }
        }
        for (pair, selected) in [
            ([0, 1], &mut self.active_vertical),
            ([2, 3], &mut self.active_horizontal),
        ] {
            for i in pair {
                if directions[i] && !self.previous_directions[i] {
                    *selected = Some(i);
                } else if !directions[i] && *selected == Some(i) {
                    *selected = pair.into_iter().find(|j| directions[*j]);
                }
            }
        }
        let codes = [8, 2, 4, 6];
        let vertical = self.active_vertical.map(|i| codes[i]).unwrap_or(0);
        let horizontal = self.active_horizontal.map(|i| codes[i]).unwrap_or(0);
        let diagonal = match (vertical, horizontal) {
            (8, 4) => 7,
            (8, 6) => 9,
            (2, 4) => 1,
            (2, 6) => 3,
            (0, h) => h,
            (v, _) => v,
        };
        result.logical.directions = [
            self.active_direction.map(|i| codes[i]).unwrap_or(0),
            diagonal,
            i32::from(directions[0]) * 8,
            i32::from(directions[1]) * 2,
            i32::from(directions[2]) * 4,
            i32::from(directions[3]) * 6,
            vertical,
            horizontal,
        ];
        // Until repeat configuration is connected, publish only verified first presses.
        if directions
            .iter()
            .zip(self.previous_directions)
            .any(|(now, old)| *now && !old)
        {
            result.logical.waiting_directions = result.logical.directions;
        }
        // 9BC848..925 combines configured keys/pad masks with permanent aliases.
        result.logical.buttons = configured_buttons(&result, bindings)
            | u8::from(down(28) || down(57) || down(156))
            | (u8::from(down(82) || down(210) || down(1) || down(14)) << 1)
            | (u8::from(down(42) || down(54)) << 2);
        result.logical.buttons &= policy.logical & 7;
        result.logical.buttons |= policy.simulated & 7;
        self.previous_directions = directions;
        result
    }
}

fn configured_buttons(sample: &KeyboardSample, bindings: ([i32; 3], [i32; 3])) -> u8 {
    let mut buttons = 0;
    for i in 0..3 {
        let key = bindings.0[i];
        let keyboard = (100..=355).contains(&key) && {
            let index = (key - 100) as usize;
            sample.down[index / 64] & (1u64 << (index % 64)) != 0
        };
        let pad = bindings.1[i];
        let gamepad = (301..=320).contains(&pad) && sample.pad_buttons & (1u32 << (pad - 297)) != 0;
        if keyboard || gamepad {
            buttons |= 1 << i;
        }
    }
    buttons
}

// PC/DirectInput index convention. Platform translation is an explicit adapter policy;
// Common22/database mapping, rather than this table, determines game actions.
fn index(key: KeyCode) -> Option<u8> {
    use KeyCode::*;
    Some(match key {
        Escape => 1,
        Digit1 => 2,
        Digit2 => 3,
        Digit3 => 4,
        Digit4 => 5,
        Digit5 => 6,
        Digit6 => 7,
        Digit7 => 8,
        Digit8 => 9,
        Digit9 => 10,
        Digit0 => 11,
        Minus => 12,
        Equal => 13,
        Backspace => 14,
        Tab => 15,
        KeyQ => 16,
        KeyW => 17,
        KeyE => 18,
        KeyR => 19,
        KeyT => 20,
        KeyY => 21,
        KeyU => 22,
        KeyI => 23,
        KeyO => 24,
        KeyP => 25,
        BracketLeft => 26,
        BracketRight => 27,
        Enter => 28,
        ControlLeft => 29,
        KeyA => 30,
        KeyS => 31,
        KeyD => 32,
        KeyF => 33,
        KeyG => 34,
        KeyH => 35,
        KeyJ => 36,
        KeyK => 37,
        KeyL => 38,
        Semicolon => 39,
        Quote => 40,
        Backquote => 41,
        ShiftLeft => 42,
        Backslash => 43,
        KeyZ => 44,
        KeyX => 45,
        KeyC => 46,
        KeyV => 47,
        KeyB => 48,
        KeyN => 49,
        KeyM => 50,
        Comma => 51,
        Period => 52,
        Slash => 53,
        ShiftRight => 54,
        NumpadMultiply => 55,
        AltLeft => 56,
        Space => 57,
        CapsLock => 58,
        F1 => 59,
        F2 => 60,
        F3 => 61,
        F4 => 62,
        F5 => 63,
        F6 => 64,
        F7 => 65,
        F8 => 66,
        F9 => 67,
        F10 => 68,
        NumLock => 69,
        ScrollLock => 70,
        Numpad7 => 71,
        Numpad8 => 72,
        Numpad9 => 73,
        NumpadSubtract => 74,
        Numpad4 => 75,
        Numpad5 => 76,
        Numpad6 => 77,
        NumpadAdd => 78,
        Numpad1 => 79,
        Numpad2 => 80,
        Numpad3 => 81,
        Numpad0 => 82,
        NumpadDecimal => 83,
        F11 => 87,
        F12 => 88,
        NumpadEnter => 156,
        ControlRight => 157,
        NumpadDivide => 181,
        AltRight => 184,
        Home => 199,
        ArrowUp => 200,
        PageUp => 201,
        ArrowLeft => 203,
        ArrowRight => 205,
        End => 207,
        ArrowDown => 208,
        PageDown => 209,
        Insert => 210,
        Delete => 211,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn short_tap_is_sampled_once_and_focus_loss_cancels_it() {
        let mut keyboard = Keyboard::default();
        keyboard.key(KeyCode::Enter, true);
        keyboard.key(KeyCode::Enter, false);
        let sample = |keyboard: &mut Keyboard| keyboard.sample_with_pad_buttons(Default::default(), ([0;3],[0;3]), 0);
        assert_eq!(sample(&mut keyboard).logical.buttons, 1);
        assert_eq!(sample(&mut keyboard).logical.buttons, 0);
        keyboard.key(KeyCode::Enter, true);
        keyboard.key(KeyCode::Enter, false);
        keyboard.focused(false);
        keyboard.focused(true);
        assert_eq!(sample(&mut keyboard).logical.buttons, 0);
    }
    #[test]
    fn configured_bindings_drive_actions_and_keep_permanent_aliases() {
        let mut keyboard = Keyboard::default();
        keyboard.key(KeyCode::KeyX, true); // native index45 -> public145
        let policy = wolf_vm::input::InputPolicy::default();
        assert_eq!(
            keyboard
                .sample_with_bindings(policy, ([145, 144, 146], [301, 302, 303]))
                .logical
                .buttons,
            1
        );
        keyboard.key(KeyCode::KeyX, false);
        keyboard.key(KeyCode::Enter, true);
        assert_eq!(
            keyboard
                .sample_with_bindings(policy, ([145, 144, 146], [301, 302, 303]))
                .logical
                .buttons,
            1
        );
        let sample = KeyboardSample {
            pad_buttons: 1 << 23,
            ..Default::default()
        };
        assert_eq!(configured_buttons(&sample, ([0; 3], [301, 320, 303])), 2);
    }
    #[test]
    fn input_policy_gates_logical_aliases_before_direction_arbitration() {
        let mut keyboard = Keyboard::default();
        keyboard.key(KeyCode::ArrowUp, true);
        keyboard.key(KeyCode::ArrowRight, true);
        keyboard.key(KeyCode::Enter, true);
        let mut policy = wolf_vm::input::InputPolicy::default();
        policy.update(0x241); // Disable confirm and right, retaining up.
        let sample = keyboard.sample_with_policy(policy);
        assert_eq!(sample.logical.directions[0..2], [8, 8]);
        assert_eq!(sample.logical.buttons, 0);
        assert_ne!(sample.down[0] & (1 << 28), 0); // Raw input is a separate path.
        policy.update(0x141); // Event input enabled, default player controls disabled.
        let sample = keyboard.sample_with_policy(policy);
        assert_eq!(sample.logical.directions[0..2], [6, 9]);
        assert_eq!(sample.logical.buttons, 1);
    }
    #[test]
    fn native_direction_arbitration_keeps_latest_axis_and_restores_held_key() {
        let mut keyboard = Keyboard::default();
        keyboard.key(KeyCode::ArrowUp, true);
        assert_eq!(keyboard.sample().logical.directions[0..2], [8, 8]);
        keyboard.key(KeyCode::ArrowRight, true);
        assert_eq!(keyboard.sample().logical.directions[0..2], [6, 9]);
        keyboard.key(KeyCode::ArrowRight, false);
        assert_eq!(keyboard.sample().logical.directions[0..2], [8, 8]);
        keyboard.key(KeyCode::ArrowDown, true);
        assert_eq!(keyboard.sample().logical.directions[0..2], [2, 2]);
        keyboard.key(KeyCode::ArrowDown, false);
        assert_eq!(keyboard.sample().logical.directions[0..2], [8, 8]);
        keyboard.key(KeyCode::Enter, true);
        assert_eq!(keyboard.sample().logical.buttons, 1);
        keyboard.key(KeyCode::Enter, false);
        keyboard.key(KeyCode::KeyZ, true);
        assert_eq!(keyboard.sample().logical.buttons, 0); // Configurable actions aren't guessed.
    }
    #[test]
    fn release_and_focus_loss_do_not_leave_stuck_bits() {
        let mut keyboard = Keyboard::default();
        keyboard.key(KeyCode::KeyA, true);
        keyboard.key(KeyCode::KeyA, true); // OS repeat cannot create an extra edge.
        assert_eq!(keyboard.sample().down[0], 1 << 30);
        keyboard.key(KeyCode::KeyA, false);
        assert_eq!(keyboard.sample().down, [0; 4]);
        keyboard.key(KeyCode::ArrowUp, true);
        keyboard.focused(false);
        assert_eq!(keyboard.sample().down, [0; 4]);
        assert!(keyboard.sample().blocked);
        keyboard.focused(true);
        assert_eq!(keyboard.sample(), KeyboardSample::default());
    }
    #[test]
    fn isolated_pad_sample_is_applied_before_logical_button_mapping() {
        let mut keyboard = Keyboard::default();
        let policy = wolf_vm::input::InputPolicy::default();
        let bindings = ([144, 145, 146], [301, 302, 303]);
        assert_eq!(
            keyboard
                .sample_with_pad_buttons(policy, bindings, 1 << 4)
                .logical
                .buttons,
            1
        );
        assert_eq!(
            keyboard
                .sample_with_pad_buttons(policy, bindings, 0)
                .logical
                .buttons,
            0
        );
        keyboard.key(KeyCode::Enter, true);
        assert_eq!(
            keyboard
                .sample_with_pad_buttons(policy, bindings, 0)
                .logical
                .buttons,
            1
        );
    }
}
