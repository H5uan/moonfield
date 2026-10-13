//! Keyboard modifier state, backend-agnostic.
//!
//! Bevy tracks no public modifier resource — shortcuts there are expressed
//! as key combinations over `ButtonInput<KeyCode>` (e.g. `ControlLeft` +
//! `KeyS`), and that works in Moonfield too since modifier keys arrive as
//! ordinary [`KeyCode`](crate::KeyCode) presses. [`Modifiers`] is an
//! additional convenience layer the backend maintains from the OS-level
//! modifiers-changed event, so "is Ctrl held" is one query instead of four.

use std::ops::{BitOr, BitOrAssign};

/// State of the keyboard modifier keys (Shift / Ctrl / Alt / Super).
///
/// A tiny manual bitflags type — no external dependency.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Modifiers(u8);

impl Modifiers {
    /// No modifiers held.
    pub const NONE: Self = Self(0);
    /// Either Shift key.
    pub const SHIFT: Self = Self(0b0001);
    /// Either Ctrl key.
    pub const CONTROL: Self = Self(0b0010);
    /// Either Alt key.
    pub const ALT: Self = Self(0b0100);
    /// Either Super key (Windows key / Cmd).
    pub const SUPER: Self = Self(0b1000);

    /// The empty set of modifiers.
    pub const fn empty() -> Self {
        Self::NONE
    }

    /// True if no modifiers are held.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True if every modifier in `other` is held.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// True if a Shift key is held.
    pub const fn shift(self) -> bool {
        self.contains(Self::SHIFT)
    }

    /// True if a Ctrl key is held.
    pub const fn control(self) -> bool {
        self.contains(Self::CONTROL)
    }

    /// True if an Alt key is held.
    pub const fn alt(self) -> bool {
        self.contains(Self::ALT)
    }

    /// True if a Super key (Windows / Cmd) is held.
    pub const fn super_key(self) -> bool {
        self.contains(Self::SUPER)
    }
}

impl BitOr for Modifiers {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_combine_and_query() {
        let mut m = Modifiers::empty();
        assert!(m.is_empty());
        m |= Modifiers::SHIFT | Modifiers::CONTROL;
        assert!(m.shift());
        assert!(m.control());
        assert!(!m.alt());
        assert!(m.contains(Modifiers::SHIFT | Modifiers::CONTROL));
        assert!(!m.contains(Modifiers::ALT));
    }
}
