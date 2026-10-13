//! Converters from winit input types to `moonfield-window`'s mirror types.
//!
//! The match bodies are mirrored 1:1 from `bevy_winit::converters`
//! (`convert_physical_key_code` / `convert_mouse_button`), so new winit key
//! variants slot in mechanically.

use moonfield_window::{KeyCode, Modifiers, MouseButton, NativeKeyCode};

/// Converts a [`winit::keyboard::NativeKeyCode`] to a Moonfield [`NativeKeyCode`].
pub fn convert_physical_native_key_code(
    native_key_code: winit::keyboard::NativeKeyCode,
) -> NativeKeyCode {
    match native_key_code {
        winit::keyboard::NativeKeyCode::Unidentified => NativeKeyCode::Unidentified,
        winit::keyboard::NativeKeyCode::Android(scan_code) => NativeKeyCode::Android(scan_code),
        winit::keyboard::NativeKeyCode::MacOS(scan_code) => NativeKeyCode::MacOS(scan_code),
        winit::keyboard::NativeKeyCode::Windows(scan_code) => NativeKeyCode::Windows(scan_code),
        winit::keyboard::NativeKeyCode::Xkb(key_code) => NativeKeyCode::Xkb(key_code),
    }
}

/// Converts a [`winit::keyboard::PhysicalKey`] to a Moonfield [`KeyCode`].
pub fn convert_physical_key_code(virtual_key_code: winit::keyboard::PhysicalKey) -> KeyCode {
    match virtual_key_code {
        winit::keyboard::PhysicalKey::Unidentified(native_key_code) => {
            KeyCode::Unidentified(convert_physical_native_key_code(native_key_code))
        }
        winit::keyboard::PhysicalKey::Code(code) => match code {
            winit::keyboard::KeyCode::Backquote => KeyCode::Backquote,
            winit::keyboard::KeyCode::Backslash => KeyCode::Backslash,
            winit::keyboard::KeyCode::BracketLeft => KeyCode::BracketLeft,
            winit::keyboard::KeyCode::BracketRight => KeyCode::BracketRight,
            winit::keyboard::KeyCode::Comma => KeyCode::Comma,
            winit::keyboard::KeyCode::Digit0 => KeyCode::Digit0,
            winit::keyboard::KeyCode::Digit1 => KeyCode::Digit1,
            winit::keyboard::KeyCode::Digit2 => KeyCode::Digit2,
            winit::keyboard::KeyCode::Digit3 => KeyCode::Digit3,
            winit::keyboard::KeyCode::Digit4 => KeyCode::Digit4,
            winit::keyboard::KeyCode::Digit5 => KeyCode::Digit5,
            winit::keyboard::KeyCode::Digit6 => KeyCode::Digit6,
            winit::keyboard::KeyCode::Digit7 => KeyCode::Digit7,
            winit::keyboard::KeyCode::Digit8 => KeyCode::Digit8,
            winit::keyboard::KeyCode::Digit9 => KeyCode::Digit9,
            winit::keyboard::KeyCode::Equal => KeyCode::Equal,
            winit::keyboard::KeyCode::IntlBackslash => KeyCode::IntlBackslash,
            winit::keyboard::KeyCode::IntlRo => KeyCode::IntlRo,
            winit::keyboard::KeyCode::IntlYen => KeyCode::IntlYen,
            winit::keyboard::KeyCode::KeyA => KeyCode::KeyA,
            winit::keyboard::KeyCode::KeyB => KeyCode::KeyB,
            winit::keyboard::KeyCode::KeyC => KeyCode::KeyC,
            winit::keyboard::KeyCode::KeyD => KeyCode::KeyD,
            winit::keyboard::KeyCode::KeyE => KeyCode::KeyE,
            winit::keyboard::KeyCode::KeyF => KeyCode::KeyF,
            winit::keyboard::KeyCode::KeyG => KeyCode::KeyG,
            winit::keyboard::KeyCode::KeyH => KeyCode::KeyH,
            winit::keyboard::KeyCode::KeyI => KeyCode::KeyI,
            winit::keyboard::KeyCode::KeyJ => KeyCode::KeyJ,
            winit::keyboard::KeyCode::KeyK => KeyCode::KeyK,
            winit::keyboard::KeyCode::KeyL => KeyCode::KeyL,
            winit::keyboard::KeyCode::KeyM => KeyCode::KeyM,
            winit::keyboard::KeyCode::KeyN => KeyCode::KeyN,
            winit::keyboard::KeyCode::KeyO => KeyCode::KeyO,
            winit::keyboard::KeyCode::KeyP => KeyCode::KeyP,
            winit::keyboard::KeyCode::KeyQ => KeyCode::KeyQ,
            winit::keyboard::KeyCode::KeyR => KeyCode::KeyR,
            winit::keyboard::KeyCode::KeyS => KeyCode::KeyS,
            winit::keyboard::KeyCode::KeyT => KeyCode::KeyT,
            winit::keyboard::KeyCode::KeyU => KeyCode::KeyU,
            winit::keyboard::KeyCode::KeyV => KeyCode::KeyV,
            winit::keyboard::KeyCode::KeyW => KeyCode::KeyW,
            winit::keyboard::KeyCode::KeyX => KeyCode::KeyX,
            winit::keyboard::KeyCode::KeyY => KeyCode::KeyY,
            winit::keyboard::KeyCode::KeyZ => KeyCode::KeyZ,
            winit::keyboard::KeyCode::Minus => KeyCode::Minus,
            winit::keyboard::KeyCode::Period => KeyCode::Period,
            winit::keyboard::KeyCode::Quote => KeyCode::Quote,
            winit::keyboard::KeyCode::Semicolon => KeyCode::Semicolon,
            winit::keyboard::KeyCode::Slash => KeyCode::Slash,
            winit::keyboard::KeyCode::AltLeft => KeyCode::AltLeft,
            winit::keyboard::KeyCode::AltRight => KeyCode::AltRight,
            winit::keyboard::KeyCode::Backspace => KeyCode::Backspace,
            winit::keyboard::KeyCode::CapsLock => KeyCode::CapsLock,
            winit::keyboard::KeyCode::ContextMenu => KeyCode::ContextMenu,
            winit::keyboard::KeyCode::ControlLeft => KeyCode::ControlLeft,
            winit::keyboard::KeyCode::ControlRight => KeyCode::ControlRight,
            winit::keyboard::KeyCode::Enter => KeyCode::Enter,
            winit::keyboard::KeyCode::SuperLeft => KeyCode::SuperLeft,
            winit::keyboard::KeyCode::SuperRight => KeyCode::SuperRight,
            winit::keyboard::KeyCode::ShiftLeft => KeyCode::ShiftLeft,
            winit::keyboard::KeyCode::ShiftRight => KeyCode::ShiftRight,
            winit::keyboard::KeyCode::Space => KeyCode::Space,
            winit::keyboard::KeyCode::Tab => KeyCode::Tab,
            winit::keyboard::KeyCode::Convert => KeyCode::Convert,
            winit::keyboard::KeyCode::KanaMode => KeyCode::KanaMode,
            winit::keyboard::KeyCode::Lang1 => KeyCode::Lang1,
            winit::keyboard::KeyCode::Lang2 => KeyCode::Lang2,
            winit::keyboard::KeyCode::Lang3 => KeyCode::Lang3,
            winit::keyboard::KeyCode::Lang4 => KeyCode::Lang4,
            winit::keyboard::KeyCode::Lang5 => KeyCode::Lang5,
            winit::keyboard::KeyCode::NonConvert => KeyCode::NonConvert,
            winit::keyboard::KeyCode::Delete => KeyCode::Delete,
            winit::keyboard::KeyCode::End => KeyCode::End,
            winit::keyboard::KeyCode::Help => KeyCode::Help,
            winit::keyboard::KeyCode::Home => KeyCode::Home,
            winit::keyboard::KeyCode::Insert => KeyCode::Insert,
            winit::keyboard::KeyCode::PageDown => KeyCode::PageDown,
            winit::keyboard::KeyCode::PageUp => KeyCode::PageUp,
            winit::keyboard::KeyCode::ArrowDown => KeyCode::ArrowDown,
            winit::keyboard::KeyCode::ArrowLeft => KeyCode::ArrowLeft,
            winit::keyboard::KeyCode::ArrowRight => KeyCode::ArrowRight,
            winit::keyboard::KeyCode::ArrowUp => KeyCode::ArrowUp,
            winit::keyboard::KeyCode::NumLock => KeyCode::NumLock,
            winit::keyboard::KeyCode::Numpad0 => KeyCode::Numpad0,
            winit::keyboard::KeyCode::Numpad1 => KeyCode::Numpad1,
            winit::keyboard::KeyCode::Numpad2 => KeyCode::Numpad2,
            winit::keyboard::KeyCode::Numpad3 => KeyCode::Numpad3,
            winit::keyboard::KeyCode::Numpad4 => KeyCode::Numpad4,
            winit::keyboard::KeyCode::Numpad5 => KeyCode::Numpad5,
            winit::keyboard::KeyCode::Numpad6 => KeyCode::Numpad6,
            winit::keyboard::KeyCode::Numpad7 => KeyCode::Numpad7,
            winit::keyboard::KeyCode::Numpad8 => KeyCode::Numpad8,
            winit::keyboard::KeyCode::Numpad9 => KeyCode::Numpad9,
            winit::keyboard::KeyCode::NumpadAdd => KeyCode::NumpadAdd,
            winit::keyboard::KeyCode::NumpadBackspace => KeyCode::NumpadBackspace,
            winit::keyboard::KeyCode::NumpadClear => KeyCode::NumpadClear,
            winit::keyboard::KeyCode::NumpadClearEntry => KeyCode::NumpadClearEntry,
            winit::keyboard::KeyCode::NumpadComma => KeyCode::NumpadComma,
            winit::keyboard::KeyCode::NumpadDecimal => KeyCode::NumpadDecimal,
            winit::keyboard::KeyCode::NumpadDivide => KeyCode::NumpadDivide,
            winit::keyboard::KeyCode::NumpadEnter => KeyCode::NumpadEnter,
            winit::keyboard::KeyCode::NumpadEqual => KeyCode::NumpadEqual,
            winit::keyboard::KeyCode::NumpadHash => KeyCode::NumpadHash,
            winit::keyboard::KeyCode::NumpadMemoryAdd => KeyCode::NumpadMemoryAdd,
            winit::keyboard::KeyCode::NumpadMemoryClear => KeyCode::NumpadMemoryClear,
            winit::keyboard::KeyCode::NumpadMemoryRecall => KeyCode::NumpadMemoryRecall,
            winit::keyboard::KeyCode::NumpadMemoryStore => KeyCode::NumpadMemoryStore,
            winit::keyboard::KeyCode::NumpadMemorySubtract => KeyCode::NumpadMemorySubtract,
            winit::keyboard::KeyCode::NumpadMultiply => KeyCode::NumpadMultiply,
            winit::keyboard::KeyCode::NumpadParenLeft => KeyCode::NumpadParenLeft,
            winit::keyboard::KeyCode::NumpadParenRight => KeyCode::NumpadParenRight,
            winit::keyboard::KeyCode::NumpadStar => KeyCode::NumpadStar,
            winit::keyboard::KeyCode::NumpadSubtract => KeyCode::NumpadSubtract,
            winit::keyboard::KeyCode::Escape => KeyCode::Escape,
            winit::keyboard::KeyCode::Fn => KeyCode::Fn,
            winit::keyboard::KeyCode::FnLock => KeyCode::FnLock,
            winit::keyboard::KeyCode::PrintScreen => KeyCode::PrintScreen,
            winit::keyboard::KeyCode::ScrollLock => KeyCode::ScrollLock,
            winit::keyboard::KeyCode::Pause => KeyCode::Pause,
            winit::keyboard::KeyCode::BrowserBack => KeyCode::BrowserBack,
            winit::keyboard::KeyCode::BrowserFavorites => KeyCode::BrowserFavorites,
            winit::keyboard::KeyCode::BrowserForward => KeyCode::BrowserForward,
            winit::keyboard::KeyCode::BrowserHome => KeyCode::BrowserHome,
            winit::keyboard::KeyCode::BrowserRefresh => KeyCode::BrowserRefresh,
            winit::keyboard::KeyCode::BrowserSearch => KeyCode::BrowserSearch,
            winit::keyboard::KeyCode::BrowserStop => KeyCode::BrowserStop,
            winit::keyboard::KeyCode::Eject => KeyCode::Eject,
            winit::keyboard::KeyCode::LaunchApp1 => KeyCode::LaunchApp1,
            winit::keyboard::KeyCode::LaunchApp2 => KeyCode::LaunchApp2,
            winit::keyboard::KeyCode::LaunchMail => KeyCode::LaunchMail,
            winit::keyboard::KeyCode::MediaPlayPause => KeyCode::MediaPlayPause,
            winit::keyboard::KeyCode::MediaSelect => KeyCode::MediaSelect,
            winit::keyboard::KeyCode::MediaStop => KeyCode::MediaStop,
            winit::keyboard::KeyCode::MediaTrackNext => KeyCode::MediaTrackNext,
            winit::keyboard::KeyCode::MediaTrackPrevious => KeyCode::MediaTrackPrevious,
            winit::keyboard::KeyCode::Power => KeyCode::Power,
            winit::keyboard::KeyCode::Sleep => KeyCode::Sleep,
            winit::keyboard::KeyCode::AudioVolumeDown => KeyCode::AudioVolumeDown,
            winit::keyboard::KeyCode::AudioVolumeMute => KeyCode::AudioVolumeMute,
            winit::keyboard::KeyCode::AudioVolumeUp => KeyCode::AudioVolumeUp,
            winit::keyboard::KeyCode::WakeUp => KeyCode::WakeUp,
            winit::keyboard::KeyCode::Meta => KeyCode::Meta,
            winit::keyboard::KeyCode::Hyper => KeyCode::Hyper,
            winit::keyboard::KeyCode::Turbo => KeyCode::Turbo,
            winit::keyboard::KeyCode::Abort => KeyCode::Abort,
            winit::keyboard::KeyCode::Resume => KeyCode::Resume,
            winit::keyboard::KeyCode::Suspend => KeyCode::Suspend,
            winit::keyboard::KeyCode::Again => KeyCode::Again,
            winit::keyboard::KeyCode::Copy => KeyCode::Copy,
            winit::keyboard::KeyCode::Cut => KeyCode::Cut,
            winit::keyboard::KeyCode::Find => KeyCode::Find,
            winit::keyboard::KeyCode::Open => KeyCode::Open,
            winit::keyboard::KeyCode::Paste => KeyCode::Paste,
            winit::keyboard::KeyCode::Props => KeyCode::Props,
            winit::keyboard::KeyCode::Select => KeyCode::Select,
            winit::keyboard::KeyCode::Undo => KeyCode::Undo,
            winit::keyboard::KeyCode::Hiragana => KeyCode::Hiragana,
            winit::keyboard::KeyCode::Katakana => KeyCode::Katakana,
            winit::keyboard::KeyCode::F1 => KeyCode::F1,
            winit::keyboard::KeyCode::F2 => KeyCode::F2,
            winit::keyboard::KeyCode::F3 => KeyCode::F3,
            winit::keyboard::KeyCode::F4 => KeyCode::F4,
            winit::keyboard::KeyCode::F5 => KeyCode::F5,
            winit::keyboard::KeyCode::F6 => KeyCode::F6,
            winit::keyboard::KeyCode::F7 => KeyCode::F7,
            winit::keyboard::KeyCode::F8 => KeyCode::F8,
            winit::keyboard::KeyCode::F9 => KeyCode::F9,
            winit::keyboard::KeyCode::F10 => KeyCode::F10,
            winit::keyboard::KeyCode::F11 => KeyCode::F11,
            winit::keyboard::KeyCode::F12 => KeyCode::F12,
            winit::keyboard::KeyCode::F13 => KeyCode::F13,
            winit::keyboard::KeyCode::F14 => KeyCode::F14,
            winit::keyboard::KeyCode::F15 => KeyCode::F15,
            winit::keyboard::KeyCode::F16 => KeyCode::F16,
            winit::keyboard::KeyCode::F17 => KeyCode::F17,
            winit::keyboard::KeyCode::F18 => KeyCode::F18,
            winit::keyboard::KeyCode::F19 => KeyCode::F19,
            winit::keyboard::KeyCode::F20 => KeyCode::F20,
            winit::keyboard::KeyCode::F21 => KeyCode::F21,
            winit::keyboard::KeyCode::F22 => KeyCode::F22,
            winit::keyboard::KeyCode::F23 => KeyCode::F23,
            winit::keyboard::KeyCode::F24 => KeyCode::F24,
            winit::keyboard::KeyCode::F25 => KeyCode::F25,
            winit::keyboard::KeyCode::F26 => KeyCode::F26,
            winit::keyboard::KeyCode::F27 => KeyCode::F27,
            winit::keyboard::KeyCode::F28 => KeyCode::F28,
            winit::keyboard::KeyCode::F29 => KeyCode::F29,
            winit::keyboard::KeyCode::F30 => KeyCode::F30,
            winit::keyboard::KeyCode::F31 => KeyCode::F31,
            winit::keyboard::KeyCode::F32 => KeyCode::F32,
            winit::keyboard::KeyCode::F33 => KeyCode::F33,
            winit::keyboard::KeyCode::F34 => KeyCode::F34,
            winit::keyboard::KeyCode::F35 => KeyCode::F35,
            _ => KeyCode::Unidentified(NativeKeyCode::Unidentified),
        },
    }
}

/// Converts a [`winit::event::MouseButton`] to a Moonfield [`MouseButton`].
pub fn convert_mouse_button(mouse_button: winit::event::MouseButton) -> MouseButton {
    match mouse_button {
        winit::event::MouseButton::Left => MouseButton::Left,
        winit::event::MouseButton::Right => MouseButton::Right,
        winit::event::MouseButton::Middle => MouseButton::Middle,
        winit::event::MouseButton::Back => MouseButton::Back,
        winit::event::MouseButton::Forward => MouseButton::Forward,
        winit::event::MouseButton::Other(val) => MouseButton::Other(val),
    }
}

/// Converts a [`winit::keyboard::ModifiersState`] to Moonfield [`Modifiers`].
pub fn convert_modifiers(state: winit::keyboard::ModifiersState) -> Modifiers {
    let mut m = Modifiers::empty();
    if state.shift_key() {
        m |= Modifiers::SHIFT;
    }
    if state.control_key() {
        m |= Modifiers::CONTROL;
    }
    if state.alt_key() {
        m |= Modifiers::ALT;
    }
    if state.super_key() {
        m |= Modifiers::SUPER;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_physical_key_code() {
        assert_eq!(
            convert_physical_key_code(winit::keyboard::PhysicalKey::Code(
                winit::keyboard::KeyCode::KeyW
            )),
            KeyCode::KeyW
        );
        assert_eq!(
            convert_physical_key_code(winit::keyboard::PhysicalKey::Unidentified(
                winit::keyboard::NativeKeyCode::Windows(0x1E)
            )),
            KeyCode::Unidentified(NativeKeyCode::Windows(0x1E))
        );
    }

    #[test]
    fn converts_mouse_button() {
        assert_eq!(
            convert_mouse_button(winit::event::MouseButton::Back),
            MouseButton::Back
        );
        assert_eq!(
            convert_mouse_button(winit::event::MouseButton::Other(7)),
            MouseButton::Other(7)
        );
    }
}
