use super::Event;
use std::sync::mpsc::Sender;
use webview2_com::{
    IsDocumentPlayingAudioChangedEventHandler, Microsoft::Web::WebView2::Win32::ICoreWebView2_8,
};
use windows::core::{BOOL, Interface};
use wry::WebViewExtWindows;

fn core(view: &wry::WebView) -> windows::core::Result<ICoreWebView2_8> {
    unsafe { view.controller().CoreWebView2()?.cast() }
}

pub(super) fn set_muted(view: &wry::WebView, muted: bool) -> windows::core::Result<()> {
    unsafe { core(view)?.SetIsMuted(muted) }
}

pub(super) fn attach(
    view: &wry::WebView,
    muted: bool,
    sender: Sender<Event>,
) -> windows::core::Result<()> {
    let audio = core(view)?;
    unsafe {
        audio.SetIsMuted(muted)?;
        audio.add_IsDocumentPlayingAudioChanged(
            &IsDocumentPlayingAudioChangedEventHandler::create(Box::new(move |source, _| {
                if let Some(source) = source {
                    let audio: ICoreWebView2_8 = source.cast()?;
                    let mut playing = BOOL::default();
                    audio.IsDocumentPlayingAudio(&mut playing)?;
                    let _ = sender.send(Event::PlayingAudio(playing.as_bool()));
                }
                Ok(())
            })),
            &mut 0,
        )
    }
}
