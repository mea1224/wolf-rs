//! GamePro 957140 keyboard sampling. Indices are native device indices, not Unicode.
//! The platform translates its key codes, pad masks and configured logical input.
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyboardSample {
    pub down: [u64; 4],
    /// Host inhibition mapped to the keyboard branch of A077D0.
    pub blocked: bool,
    /// GamePro 1557F78 native pad mask. Buttons 301..315 occupy bits 4..18.
    #[serde(default)]
    pub pad_buttons: u32,
    #[serde(default)]
    pub pad_connected: bool,
    /// Host publishes engine logical state after applying its actual input configuration.
    #[serde(default)]
    pub logical: LogicalSample,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LogicalSample {
    /// Selectors 1..8: four/eight directions, up/down/left/right, vertical/horizontal.
    pub directions: [i32; 8],
    /// Native directional repeat/trigger values used by waiting modes 0/1.
    pub waiting_directions: [i32; 8],
    /// Bits 0,1,2 are engine action 10,11,12, not physical keys.
    pub buttons: u8,
    /// Native repeat pulse for actions 10/11; ordinary new presses are detected here.
    pub button_repeat: u8,
    pub repeat_enabled: u8,
}
impl KeyboardSample {
    pub fn set(&mut self, index: u8, down: bool) {
        let word = &mut self.down[usize::from(index) / 64];
        let mask = 1u64 << (index % 64);
        if down {
            *word |= mask;
        } else {
            *word &= !mask;
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyboardState {
    pub sample: KeyboardSample,
    pub previous: [u64; 4],
    #[serde(default)]
    pub previous_pad: u32,
    #[serde(default)]
    pub previous_logical: LogicalSample,
    #[serde(default)]
    pub policy: InputPolicy,
}

/// A18380 low mode: physical logical input and default player processing are
/// separate. Flag bits map confirm/cancel/action3/down/left/right/up.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InputPolicy {
    pub logical: u8,
    pub player: u8,
    /// A18190 / 9BBE0B / 9BC83A: forced buttons bypass physical gates;
    /// 9BD6E0 clears them after the next device update.
    pub simulated: u8,
}
impl Default for InputPolicy {
    fn default() -> Self {
        Self {
            logical: 0xf7,
            player: 0xf7,
            simulated: 0,
        }
    }
}
impl InputPolicy {
    pub fn update(&mut self, flags: i32) {
        let selected = flags as u8 & 0xf7;
        self.logical = (self.logical & !selected) | if flags & 0x200 == 0 { selected } else { 0 };
        self.player = (self.player & !selected) | if flags & 0x300 == 0 { selected } else { 0 };
    }
}
impl KeyboardState {
    fn bit(words: &[u64; 4], index: usize) -> bool {
        words[index / 64] & (1u64 << (index % 64)) != 0
    }
    fn matches(&self, index: usize, mode: u8) -> bool {
        let now = Self::bit(&self.sample.down, index);
        let old = Self::bit(&self.previous, index);
        match mode {
            0 | 1 => now,
            2 => now && !old,
            3 => !now && old,
            _ => false,
        }
    }
    /// The initialization call uses scan mode 0 and records a newly pressed index.
    pub(crate) fn initialize_wait(&self) -> i32 {
        let mut memory = 0;
        self.scan(0, true, true, &mut memory);
        memory
    }
    fn scan(&self, mode: u8, waiting: bool, initialize: bool, memory: &mut i32) -> i32 {
        for index in 2..=146 {
            if [28, 42, 54, 14, 57, 58, 112].contains(&index) || !self.matches(index, mode) {
                continue;
            }
            if !waiting {
                return index as i32 + 100;
            }
            if *memory != index as i32 && !Self::bit(&self.previous, index) {
                *memory = if initialize { index as i32 } else { 0 };
                return index as i32 + 100;
            }
        }
        *memory = 0;
        0
    }
    pub(crate) fn sample(&self, key: i32, mode: u8, waiting: bool, memory: &mut i32) -> i32 {
        if (100..=355).contains(&key) {
            let index = (key - 100) as usize;
            if self.matches(index, mode)
                && !(waiting && mode <= 1 && Self::bit(&self.previous, index))
            {
                key
            } else {
                0
            }
        } else {
            self.scan(mode, waiting, false, memory)
        }
    }
    pub(crate) fn finish_tick(&mut self) {
        self.previous = self.sample.down;
        self.previous_pad = self.sample.pad_buttons;
        self.previous_logical = self.sample.logical;
    }

    /// A14B50 initializes the pad wait memory by scanning in mode 0.
    pub(crate) fn initialize_command(&self, flags: i32) -> i32 {
        let mut memory = 0;
        if flags & 0xf00 == 0x200 {
            self.pad(0, 0, true, true, &mut memory);
        }
        memory
    }

    pub(crate) fn sample_command(
        &self,
        flags: i32,
        key: i32,
        waiting: bool,
        memory: &mut i32,
    ) -> i32 {
        let mode = ((flags >> 16) & 15) as u8;
        match flags & 0xf00 {
            0x200 => self.pad(key, mode, waiting, false, memory),
            0 => self.logical(flags, mode, waiting),
            _ => 0, // IR validation prevents an unsupported device reaching this method.
        }
    }

    fn pad(&self, key: i32, mode: u8, waiting: bool, initialize: bool, memory: &mut i32) -> i32 {
        let now = self.sample.pad_buttons;
        let old = self.previous_pad;
        if !waiting && !initialize {
            *memory = 0;
        }
        let matched = |mask| match mode {
            0 | 1 => now & mask != 0,
            2 => now & mask != 0 && old & mask == 0,
            3 => now & mask == 0 && old & mask != 0,
            _ => false,
        };
        // 957330 tests 301..320, but its map actually contains only 301..315.
        if (301..=320).contains(&key) {
            if key > 315 {
                *memory = 0;
                return 0;
            }
            let mask = 1u32 << (key - 297);
            if mode <= 1 && waiting && old & mask != 0 {
                return 0;
            }
            if !matched(mask) {
                *memory = 0;
                return 0;
            }
            if mode <= 1 {
                if *memory == key {
                    return 0;
                }
                *memory = if initialize { key } else { 0 };
            }
            return key;
        }
        let mut held = 0;
        for key in 301..=315 {
            let mask = 1u32 << (key - 297);
            if !matched(mask) {
                continue;
            }
            if mode <= 1 {
                held = now;
                if *memory as u32 & mask != 0 {
                    continue;
                }
                *memory = if initialize { now as i32 } else { 0 };
            }
            return key;
        }
        *memory = held as i32;
        0
    }

    fn logical(&self, flags: i32, mode: u8, waiting: bool) -> i32 {
        let now = &self.sample.logical;
        let old = &self.previous_logical;
        let mut direction_allowed = true;
        for bit in 0..3 {
            let mask = 1 << bit;
            if flags & (0x10 << bit) == 0 {
                continue;
            }
            let down = now.buttons & mask != 0;
            let previous = old.buttons & mask != 0;
            let pressed = down && !previous;
            let accepted = match mode {
                0 | 1 if waiting => {
                    pressed
                        || (bit < 2
                            && now.repeat_enabled & mask != 0
                            && now.button_repeat & mask != 0
                            && direction_allowed)
                }
                0 | 1 => down,
                2 => pressed,
                3 => !down && previous,
                _ => false,
            };
            if mode <= 1 && bit < 2 {
                direction_allowed &= !(down && now.repeat_enabled & mask != 0);
            }
            if accepted {
                return 10 + bit;
            }
        }
        let selector = flags & 15;
        if selector == 0 {
            return 0;
        }
        let index = selector as usize - 1;
        let current = now.directions[index];
        let previous = old.directions[index];
        let result = match mode {
            0 | 1 if waiting && (3..=6).contains(&selector) => {
                if (current != 0 && now.waiting_directions[0] != 0 && direction_allowed)
                    || (current != 0 && previous == 0)
                {
                    current
                } else {
                    0
                }
            }
            0 | 1 if waiting => {
                if direction_allowed {
                    now.waiting_directions[index]
                } else {
                    0
                }
            }
            0 | 1 => current,
            2 if current != previous => current,
            3 if current == 0 => previous,
            _ => 0,
        };
        if result == 2_000_000_000 {
            0
        } else {
            result
        }
    }
}
