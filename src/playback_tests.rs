use super::*;
use std::{
    num::NonZeroIsize,
    sync::mpsc,
    time::{Duration, Instant},
};
use windows::{
    Win32::{Foundation::HWND, UI::WindowsAndMessaging::*},
    core::w,
};
use wry::raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};

fn sample(mime: &'static str, bytes: &[u8]) -> Source {
    Source {
        mime,
        bytes: Arc::new(bytes.to_vec()),
        failed: Default::default(),
    }
}

#[test]
fn playback_document_opens_read_only_and_keeps_original_bytes_after_removal() {
    use crate::files::Document;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("movie.WEBM");
    let bytes = include_bytes!("../tests/fixtures/player-blue.webm");
    std::fs::write(&path, bytes).unwrap();
    let mut doc = Document::open(temp.path(), &path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(doc.media && !doc.dirty());
    assert!(!doc.apply_native_edit(doc.instance_id(), "", "changed".into()));
    let source = doc.playback.as_ref().unwrap();
    assert_eq!(source.mime, "video/webm");
    assert_eq!(&source.bytes[..], bytes);
    let file = std::fs::File::create(&path).unwrap();
    file.set_len((MAX_BYTES + 1) as u64).unwrap();
    assert!(Document::open(temp.path(), &path).is_err());
}

#[test]
fn media_types_and_byte_ranges_match_original_files() {
    use wry::http::StatusCode;
    for ext in [
        "mp3", "wav", "ogg", "oga", "opus", "m4a", "aac", "flac", "weba",
    ] {
        assert!(mime(ext).unwrap().starts_with("audio/"));
    }
    for ext in ["mp4", "m4v", "webm", "ogv", "mov"] {
        assert!(mime(ext).unwrap().starts_with("video/"));
    }
    assert_eq!(mime("html"), None);
    let src = sample("video/webm", b"abcdefgh");
    for (range, expected, data) in [
        ("bytes=2-4", "bytes 2-4/8", &b"cde"[..]),
        ("bytes=2-", "bytes 2-7/8", &b"cdefgh"[..]),
        ("bytes=-2", "bytes 6-7/8", &b"gh"[..]),
        ("bytes=3-100", "bytes 3-7/8", &b"defgh"[..]),
    ] {
        let req = Request::get("tesselmedia://localhost/original")
            .header("Range", range)
            .body(vec![])
            .unwrap();
        let res = serve(&src, false, &req);
        assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(res.headers()["content-range"], expected);
        assert_eq!(res.body().as_ref(), data);
    }
    for range in ["bytes=8-", "bytes=-0", "bytes=3-1", "bytes=0-2,5-7"] {
        let req = Request::get("tesselmedia://localhost/original")
            .header("Range", range)
            .body(vec![])
            .unwrap();
        assert_eq!(serve(&src, false, &req).status(), 416);
    }
    let req = Request::head("tesselmedia://localhost/original")
        .body(vec![])
        .unwrap();
    let head = serve(&src, false, &req);
    assert_eq!(head.headers()["content-length"], "8");
    assert!(head.body().is_empty());
    let req = Request::get("tesselmedia://unrelated/original")
        .body(vec![])
        .unwrap();
    assert_eq!(serve(&src, false, &req).status(), 404);
}

struct Parent(HWND);
impl Drop for Parent {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}
impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = Win32WindowHandle::new(NonZeroIsize::new(self.0.0 as isize).unwrap());
        // The HWND belongs to Parent and outlives the borrowed handle.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(raw)) })
    }
}

fn pump() {
    let mut msg = MSG::default();
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    std::thread::sleep(Duration::from_millis(5));
}

fn wait_for(view: &WebView, expression: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut last = String::new();
    while Instant::now() < deadline {
        let (tx, rx) = mpsc::channel();
        view.evaluate_script_with_callback(expression, move |s| {
            let _ = tx.send(s);
        })
        .unwrap();
        while Instant::now() < deadline {
            pump();
            if let Ok(value) = rx.try_recv() {
                if value == "true" {
                    return;
                }
                last = value;
                break;
            }
        }
    }
    panic!("media probe timed out: {expression}; last result {last}");
}

#[test]
#[ignore = "requires installed Windows WebView2 runtime"]
fn hidden_webview_decodes_original_audio_and_video_seeks_and_releases_the_player() {
    // Hidden native parent keeps the real decoder test off the user's desktop.
    let parent = Parent(
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("Media preview test"),
                WS_POPUP | WS_CLIPCHILDREN,
                0,
                0,
                400,
                300,
                None,
                None,
                None,
                None,
            )
        }
        .unwrap(),
    );
    let mut player = Player::default();
    let bounds =
        eframe::egui::Rect::from_min_size(eframe::egui::Pos2::ZERO, eframe::egui::vec2(400., 300.));
    for (kind, bytes, tag) in [
        (
            "audio/wav",
            &include_bytes!("../tests/fixtures/player-silence.wav")[..],
            "audio",
        ),
        (
            "video/webm",
            &include_bytes!("../tests/fixtures/player-blue.webm")[..],
            "video",
        ),
    ] {
        let src = sample(kind, bytes);
        player.sync(&parent, &src, bounds, 1., false);
        let view = player.view.as_ref().expect("WebView2 player");
        wait_for(
            view,
            &format!(
                "(()=>{{const m=document.querySelector('{tag}');return m&&m.readyState>=1&&m.controls&&m.duration>0.9&&m.duration<1.1?true:[location.href,m?.readyState,m?.error?.code]}})()"
            ),
        );
        if tag == "video" {
            wait_for(
                view,
                "document.querySelector('video').videoWidth === 64 && document.querySelector('video').videoHeight === 48",
            );
        }
        view.evaluate_script("document.querySelector('audio,video').currentTime=0.4")
            .unwrap();
        wait_for(
            view,
            "(()=>{const m=document.querySelector('audio,video');return !m.seeking && Math.abs(m.currentTime-0.4)<0.05})()",
        );
        view.evaluate_script(
            "const el=document.querySelector('audio,video');el.muted=true;el.play()",
        )
        .unwrap();
        wait_for(
            view,
            "document.querySelector('audio,video').currentTime > 0.45",
        );
        view.evaluate_script("document.querySelector('audio,video').pause()")
            .unwrap();
        wait_for(view, "document.querySelector('audio,video').paused");
        player.hide();
        assert!(player.hidden);
        player.sync(&parent, &src, bounds, 1., false);
        wait_for(
            player.view.as_ref().unwrap(),
            "document.querySelector('audio,video').currentTime > 0.45 && document.querySelector('audio,video').paused",
        );
        player.retain(Some(&src));
        assert!(player.view.is_some());
    }
    let broken = sample("video/mp4", b"invalid movie");
    player.sync(&parent, &broken, bounds, 1., true);
    wait_for(
        player.view.as_ref().unwrap(),
        "document.querySelector('video')?.hidden === true && document.querySelector('span')?.textContent.includes('multimédia')",
    );
    player.retain(None);
    assert!(player.view.is_none());
}
