//! Windows owns the popup above the native WebView2 child, including dismissal and keyboard input.
use crate::i18n::tr;
use windows::{
    Win32::{
        Foundation::{HWND, POINT},
        UI::WindowsAndMessaging::*,
    },
    core::{HSTRING, Result},
};
use wry::WebViewExtWindows;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Choice {
    Reload = 1,
    HardReload = 2,
}

impl Choice {
    fn from_id(id: i32) -> Option<Self> {
        match id {
            1 => Some(Self::Reload),
            2 => Some(Self::HardReload),
            _ => None,
        }
    }
}

struct Menu(HMENU);
impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.0);
        }
    }
}

impl Menu {
    fn new() -> Result<Self> {
        let menu = Self(unsafe { CreatePopupMenu()? });
        for (choice, label, shortcut) in [
            (Choice::Reload, "Reload", "Ctrl+R"),
            (Choice::HardReload, "Hard Reload", "Ctrl+Shift+R"),
        ] {
            let label = HSTRING::from(format!("{}\t{shortcut}", tr(label)));
            unsafe {
                AppendMenuW(menu.0, MF_STRING, choice as usize, &label)?;
            }
        }
        Ok(menu)
    }
}

pub(super) fn choose(view: &wry::WebView) -> Result<Option<Choice>> {
    let menu = Menu::new()?;
    let mut owner = HWND::default();
    let mut cursor = POINT::default();
    unsafe {
        view.controller().ParentWindow(&mut owner)?;
        GetCursorPos(&mut cursor)?;
        let id = TrackPopupMenu(
            menu.0,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            cursor.x,
            cursor.y,
            None,
            owner,
            None,
        );
        Ok(Choice::from_id(id.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reload_popup_offers_both_commands_and_dismisses_without_a_selection() {
        let menu = Menu::new().unwrap();
        assert_eq!(unsafe { GetMenuItemCount(Some(menu.0)) }, 2);
        for (index, expected, shortcut) in [
            (0, Choice::Reload, "Ctrl+R"),
            (1, Choice::HardReload, "Ctrl+Shift+R"),
        ] {
            let id = unsafe { GetMenuItemID(menu.0, index) };
            assert_eq!(Choice::from_id(id as i32), Some(expected));
            let mut text = [0u16; 128];
            let len = unsafe { GetMenuStringW(menu.0, id, Some(&mut text), MF_BYCOMMAND) };
            assert!(String::from_utf16_lossy(&text[..len as usize]).ends_with(shortcut));
        }
        assert_eq!(Choice::from_id(0), None);
    }
}
