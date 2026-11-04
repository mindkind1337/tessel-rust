#[path = "../src/browser_clip.rs"]
mod browser_clip;
use browser_clip::{Cutout, cutout};
use eframe::egui::{Rect, pos2};

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
    Rect::from_min_max(pos2(left, top), pos2(right, bottom))
}

#[test]
fn floating_header_excludes_only_the_overlap_without_moving_page_origin() {
    let browser = rect(100.0, 120.0, 900.0, 720.0);
    assert_eq!(
        cutout(browser, Some(rect(80.0, 60.0, 920.0, 360.0)), 1.0),
        Ok(Some(Cutout {
            width: 800,
            height: 600,
            left: 0,
            top: 0,
            right: 800,
            bottom: 240,
        }))
    );
    assert_eq!(cutout(browser, None, 1.0), Ok(None));
    assert_eq!(
        cutout(browser, Some(rect(0.0, 0.0, 100.0, 120.0)), 1.0),
        Ok(None)
    );
}

#[test]
fn fractional_dpi_covers_boundary_pixels_and_clamps_all_edges() {
    let browser = rect(10.5, 20.5, 210.5, 120.5);
    assert_eq!(
        cutout(browser, Some(rect(15.1, 25.1, 100.1, 60.1)), 1.25),
        Ok(Some(Cutout {
            width: 250,
            height: 125,
            left: 5,
            top: 6,
            right: 113,
            bottom: 51,
        }))
    );
    assert_eq!(
        cutout(browser, Some(rect(-100.0, -100.0, 500.0, 500.0)), 1.25),
        Ok(Some(Cutout {
            width: 250,
            height: 125,
            left: 0,
            top: 0,
            right: 250,
            bottom: 125,
        }))
    );
}

#[test]
fn invalid_geometry_fails_closed_and_negative_window_positions_are_supported() {
    let browser = rect(-200.0, -100.0, 100.0, 100.0);
    assert_eq!(
        cutout(browser, Some(rect(-150.0, -80.0, -100.0, -20.0)), 2.0),
        Ok(Some(Cutout {
            width: 600,
            height: 400,
            left: 100,
            top: 40,
            right: 200,
            bottom: 160,
        }))
    );
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::MAX] {
        assert!(cutout(browser, None, scale).is_err());
    }
    assert!(cutout(Rect::NAN, None, 1.0).is_err());
    assert!(cutout(rect(0.0, 0.0, 70_000_000.0, 100.0), None, 1.0).is_err());
    assert!(cutout(browser, Some(Rect::NAN), 1.0).is_err());
}

#[cfg(windows)]
mod win32_tests {
    use super::browser_clip::{Cutout, NativeClip};
    use std::{
        num::NonZeroIsize,
        time::{Duration, Instant},
    };
    use windows::{
        Win32::{
            Foundation::{HWND, POINT, RECT},
            Graphics::Gdi::{
                CreateRectRgn, DeleteObject, GetWindowRgn, HGDIOBJ, HRGN, NULLREGION, PtInRegion,
                SetWindowRgn,
            },
            System::Threading::GetCurrentProcess,
            UI::WindowsAndMessaging::{
                CWP_ALL, ChildWindowFromPointEx, CreateWindowExW, DestroyWindow, DispatchMessageW,
                GetWindowRect, IsChild, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
                WINDOW_EX_STYLE, WS_CLIPCHILDREN, WS_POPUP,
            },
        },
        core::w,
    };

    const GR_GDIOBJECTS: u32 = 0;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetGuiResources(hProcess: windows::Win32::Foundation::HANDLE, uiFlags: u32) -> u32;
    }

    struct Window(HWND);
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }

    struct Region(HRGN);
    impl Drop for Region {
        fn drop(&mut self) {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.0.0));
            }
        }
    }

    fn hidden_window(width: i32, height: i32) -> Window {
        // No WS_VISIBLE: isolated hidden window, never touches the desktop or user browser.
        Window(
            unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    w!("Tessel clipping fixture"),
                    WS_POPUP | WS_CLIPCHILDREN,
                    0,
                    0,
                    width,
                    height,
                    None,
                    None,
                    None,
                    None,
                )
            }
            .unwrap(),
        )
    }

    #[test]
    fn native_region_boundaries_cache_hit_full_occluder_and_non_poisoning() {
        let window = hidden_window(300, 200);
        let readback = Region(unsafe { CreateRectRgn(0, 0, 0, 0) });
        assert!(!readback.0.0.is_null());

        let mut before = RECT::default();
        unsafe {
            GetWindowRect(window.0, &mut before).unwrap();
        }

        let cutout = Some(Cutout {
            width: 300,
            height: 200,
            left: 0,
            top: 0,
            right: 300,
            bottom: 80,
        });
        let invalid_same_hwnd = Some(Cutout {
            width: 300,
            height: 200,
            left: 120,
            top: 0,
            right: 60,
            bottom: 80,
        });

        let mut clip = NativeClip::default();
        // 1. Failed apply with invalid HWND and with the SAME valid HWND must never poison cache.
        assert!(clip.apply(HWND::default(), cutout).is_err());
        assert!(clip.apply(window.0, invalid_same_hwnd).is_err());
        assert!(clip.apply(window.0, invalid_same_hwnd).is_err());

        // 2. Apply valid cutout and verify exact 1px boundary coordinates.
        clip.apply(window.0, cutout).unwrap();
        assert_ne!(unsafe { GetWindowRgn(window.0, readback.0) }.0, 0);
        assert!(!unsafe { PtInRegion(readback.0, 50, 0) }.as_bool());
        assert!(!unsafe { PtInRegion(readback.0, 50, 79) }.as_bool());
        assert!(unsafe { PtInRegion(readback.0, 0, 80) }.as_bool());
        assert!(unsafe { PtInRegion(readback.0, 50, 80) }.as_bool());
        assert!(unsafe { PtInRegion(readback.0, 299, 199) }.as_bool());
        assert!(!unsafe { PtInRegion(readback.0, 300, 199) }.as_bool());
        assert!(!unsafe { PtInRegion(readback.0, 299, 200) }.as_bool());

        // Window screen bounds are unchanged by SetWindowRgn.
        let mut after = RECT::default();
        unsafe {
            GetWindowRect(window.0, &mut after).unwrap();
        }
        assert_eq!(
            (before.left, before.top, before.right, before.bottom),
            (after.left, after.top, after.right, after.bottom)
        );

        // 3. Prove cache hit: clear OS region directly behind NativeClip's back; repeated apply
        //    with identical key short-circuits, while reset() forces reapplication.
        assert_ne!(unsafe { SetWindowRgn(window.0, None, true) }, 0);
        assert_eq!(unsafe { GetWindowRgn(window.0, readback.0) }.0, 0);
        clip.apply(window.0, cutout).unwrap();
        assert_eq!(
            unsafe { GetWindowRgn(window.0, readback.0) }.0,
            0,
            "identical key must hit cache without calling SetWindowRgn again"
        );
        clip.reset();
        clip.apply(window.0, cutout).unwrap();
        assert_ne!(unsafe { GetWindowRgn(window.0, readback.0) }.0, 0);

        // 4. Full-window occluder produces NULLREGION where zero browser pixels remain live.
        let full_hole = Some(Cutout {
            width: 300,
            height: 200,
            left: 0,
            top: 0,
            right: 300,
            bottom: 200,
        });
        clip.apply(window.0, full_hole).unwrap();
        assert_eq!(unsafe { GetWindowRgn(window.0, readback.0) }, NULLREGION);
        assert!(!unsafe { PtInRegion(readback.0, 0, 0) }.as_bool());
        assert!(!unsafe { PtInRegion(readback.0, 150, 100) }.as_bool());

        // 5. Failed apply after a valid apply clears cache so subsequent recovery re-runs.
        assert!(clip.apply(window.0, invalid_same_hwnd).is_err());
        assert_ne!(unsafe { SetWindowRgn(window.0, None, true) }, 0);
        clip.apply(window.0, full_hole).unwrap();
        assert_eq!(unsafe { GetWindowRgn(window.0, readback.0) }, NULLREGION);

        // Restore full unclipped window.
        clip.apply(window.0, None).unwrap();
        assert_eq!(unsafe { GetWindowRgn(window.0, readback.0) }.0, 0);
    }

    #[test]
    fn repeated_region_updates_do_not_leak_gdi_handles() {
        let window = hidden_window(300, 200);
        let mut clip = NativeClip::default();
        clip.apply(
            window.0,
            Some(Cutout {
                width: 300,
                height: 200,
                left: 0,
                top: 0,
                right: 300,
                bottom: 50,
            }),
        )
        .unwrap();

        let proc = unsafe { GetCurrentProcess() };
        let baseline = unsafe { GetGuiResources(proc, GR_GDIOBJECTS) };
        assert!(baseline > 0);

        for step in 1..=200 {
            let bottom = (step % 150) + 10;
            clip.apply(
                window.0,
                Some(Cutout {
                    width: 300,
                    height: 200,
                    left: 0,
                    top: 0,
                    right: 300,
                    bottom,
                }),
            )
            .unwrap();
        }
        let during = unsafe { GetGuiResources(proc, GR_GDIOBJECTS) };
        assert!(
            during <= baseline + 2,
            "GDI handle count grew across 200 region updates: baseline={baseline}, during={during}"
        );

        clip.apply(window.0, None).unwrap();
        let restored = unsafe { GetGuiResources(proc, GR_GDIOBJECTS) };
        assert!(
            restored <= baseline + 2,
            "GDI handle count grew after restoring unclipped window: baseline={baseline}, restored={restored}"
        );
    }

    #[test]
    fn real_webview2_child_container_clips_hit_testing_and_restores_cleanly() {
        use wry::{
            Rect as WryRect, WebContext, WebViewBuilder, WebViewExtWindows,
            dpi::{PhysicalPosition, PhysicalSize},
            raw_window_handle::{
                HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
            },
        };

        struct Parent(Window);
        impl HasWindowHandle for Parent {
            fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
                let nz = NonZeroIsize::new(self.0.0.0 as isize).unwrap();
                let raw = RawWindowHandle::Win32(Win32WindowHandle::new(nz));
                Ok(unsafe { WindowHandle::borrow_raw(raw) })
            }
        }

        fn pump_messages(duration: Duration) {
            let deadline = Instant::now() + duration;
            while Instant::now() < deadline {
                let mut msg = MSG::default();
                while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        let profile_dir = tempfile::Builder::new()
            .prefix("tessel-webview-clip-test-")
            .tempdir()
            .unwrap();
        let parent = Parent(hidden_window(300, 200));
        let mut context = WebContext::new(Some(profile_dir.path().to_path_buf()));

        let view = WebViewBuilder::new_with_web_context(&mut context)
            .with_html("<!doctype html><html><body style='margin:0'><button id='b'>ok</button></body></html>")
            .with_bounds(WryRect {
                position: PhysicalPosition::new(0, 0).into(),
                size: PhysicalSize::new(300_u32, 200_u32).into(),
            })
            .with_visible(true)
            .with_devtools(false)
            .build_as_child(&parent)
            .expect("isolated WebView2 child container should build");

        pump_messages(Duration::from_millis(120));

        let child_hwnd = view.hwnd();
        assert!(!child_hwnd.0.is_null());
        assert!(unsafe { IsChild(parent.0.0, child_hwnd) }.as_bool());

        // Unclipped WebView2 container owns both top (y=40) and bottom (y=140) child hit-test points.
        assert_eq!(
            unsafe { ChildWindowFromPointEx(parent.0.0, POINT { x: 50, y: 40 }, CWP_ALL) },
            child_hwnd
        );
        assert_eq!(
            unsafe { ChildWindowFromPointEx(parent.0.0, POINT { x: 50, y: 140 }, CWP_ALL) },
            child_hwnd
        );

        let mut clip = NativeClip::default();
        let drawer_cutout = Some(Cutout {
            width: 300,
            height: 200,
            left: 0,
            top: 0,
            right: 300,
            bottom: 80,
        });
        clip.apply(child_hwnd, drawer_cutout).unwrap();
        pump_messages(Duration::from_millis(30));

        let readback = Region(unsafe { CreateRectRgn(0, 0, 0, 0) });
        assert_ne!(unsafe { GetWindowRgn(child_hwnd, readback.0) }.0, 0);
        assert!(!unsafe { PtInRegion(readback.0, 50, 79) }.as_bool());
        assert!(unsafe { PtInRegion(readback.0, 50, 80) }.as_bool());

        // With cutout applied on the real WebView2 container, top drawer points fall through to
        // the parent host window while lower viewport points still hit WebView2.
        assert_eq!(
            unsafe { ChildWindowFromPointEx(parent.0.0, POINT { x: 50, y: 40 }, CWP_ALL) },
            parent.0.0
        );
        assert_eq!(
            unsafe { ChildWindowFromPointEx(parent.0.0, POINT { x: 50, y: 79 }, CWP_ALL) },
            parent.0.0
        );
        assert_eq!(
            unsafe { ChildWindowFromPointEx(parent.0.0, POINT { x: 50, y: 80 }, CWP_ALL) },
            child_hwnd
        );
        assert_eq!(
            unsafe { ChildWindowFromPointEx(parent.0.0, POINT { x: 50, y: 140 }, CWP_ALL) },
            child_hwnd
        );

        // Restoring `None` gives the top region back to WebView2 without resizing it.
        clip.apply(child_hwnd, None).unwrap();
        pump_messages(Duration::from_millis(20));
        assert_eq!(unsafe { GetWindowRgn(child_hwnd, readback.0) }.0, 0);
        assert_eq!(
            unsafe { ChildWindowFromPointEx(parent.0.0, POINT { x: 50, y: 40 }, CWP_ALL) },
            child_hwnd
        );
    }
}
