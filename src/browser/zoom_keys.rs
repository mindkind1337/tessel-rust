use super::Event;
use std::sync::mpsc::Sender;
use webview2_com::{
    AcceleratorKeyPressedEventHandler,
    Microsoft::Web::WebView2::Win32::COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_ADD, VK_CONTROL, VK_MENU, VK_NUMPAD0, VK_SHIFT, VK_SUBTRACT, VkKeyScanW,
};
use wry::WebViewExtWindows;

pub(super) fn attach(view: &wry::WebView, sender: Sender<Event>) -> Result<(), String> {
    // WebView2 delivers accelerators on its UI thread. The controller owns the
    // subscription; post only the zoom command to the originating tab.
    unsafe {
        view.controller().add_AcceleratorKeyPressed(
            &AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = Default::default();
                args.KeyEventKind(&mut kind)?;
                if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN
                    || GetKeyState(VK_CONTROL.0.into()) >= 0
                    || GetKeyState(VK_MENU.0.into()) < 0
                {
                    return Ok(());
                }
                let mut vk = 0;
                args.VirtualKey(&mut vk)?;
                let shifted = GetKeyState(VK_SHIFT.0.into()) < 0;
                let matches = |ch: u8| {
                    let mapped = VkKeyScanW(ch.into());
                    mapped >= 0
                        && u32::from(mapped as u16 & 0xff) == vk
                        && (mapped >> 8) == i16::from(shifted)
                };
                let direction = if vk == u32::from(VK_ADD.0) || matches(b'=') || matches(b'+') {
                    1
                } else if vk == u32::from(VK_SUBTRACT.0) || matches(b'-') {
                    -1
                } else if vk == u32::from(VK_NUMPAD0.0) || matches(b'0') {
                    0
                } else {
                    return Ok(());
                };
                if sender.send(Event::Zoom(direction)).is_ok() {
                    args.SetHandled(true)?;
                }
                Ok(())
            })),
            &mut 0,
        )
    }
    .map_err(|error| error.to_string())
}
