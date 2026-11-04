//! Geometry for excluding an egui overlay from a native child browser window.
//! This does not resize the browser viewport or change its page/scroll state.
//! Coordinates follow SetWindowRgn: physical pixels relative to the window corner.

// CreateRectRgn stores signed 27-bit region coordinates.
const MAX_REGION_COORD: i32 = (1 << 26) - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cutout {
    pub width: i32,
    pub height: i32,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Applies only to the child HWND owned by the embedded browser, on its UI thread.
/// The cached geometry avoids creating a GDI region on every repaint.
#[cfg(windows)]
#[derive(Default)]
pub struct NativeClip {
    applied: Option<(isize, Option<Cutout>)>,
}

#[cfg(windows)]
impl NativeClip {
    /// Clears cached region state so the next [`Self::apply`] unconditionally updates Win32.
    pub fn reset(&mut self) {
        self.applied = None;
    }

    /// Applies or clears the child HWND's window region.
    ///
    /// Fail-closed contract: if this returns `Err`, the previous Win32 region (or unclipped
    /// full window) may still be active on `hwnd`, and the internal cache is cleared. The caller
    /// must immediately hide the native child window rather than leaving it unclipped under an
    /// overlay.
    pub fn apply(
        &mut self,
        hwnd: windows::Win32::Foundation::HWND,
        cutout: Option<Cutout>,
    ) -> Result<(), &'static str> {
        use windows::Win32::Graphics::Gdi::{CombineRgn, RGN_DIFF, SetWindowRgn};
        let key = (hwnd.0 as isize, cutout);
        if self.applied == Some(key) {
            return Ok(());
        }
        if let Some(cutout) = cutout {
            if cutout.width > MAX_REGION_COORD
                || cutout.height > MAX_REGION_COORD
                || cutout.width <= 0
                || cutout.height <= 0
                || cutout.left < 0
                || cutout.top < 0
                || cutout.right > cutout.width
                || cutout.bottom > cutout.height
                || cutout.left >= cutout.right
                || cutout.top >= cutout.bottom
            {
                self.applied = None;
                return Err("Invalid native browser cutout");
            }
            let outer = match Region::new(0, 0, cutout.width, cutout.height) {
                Ok(region) => region,
                Err(err) => {
                    self.applied = None;
                    return Err(err);
                }
            };
            let hole = match Region::new(cutout.left, cutout.top, cutout.right, cutout.bottom) {
                Ok(region) => region,
                Err(err) => {
                    self.applied = None;
                    return Err(err);
                }
            };
            // Win32 allows the destination region to be one of the source regions.
            if unsafe { CombineRgn(Some(outer.0), Some(outer.0), Some(hole.0), RGN_DIFF) }.0 == 0 {
                self.applied = None;
                return Err("Cannot construct browser window region");
            }
            if unsafe { SetWindowRgn(hwnd, Some(outer.0), true) } == 0 {
                self.applied = None;
                return Err("Cannot apply browser window region");
            }
            // On success Windows owns this region. The temporary hole is still ours.
            std::mem::forget(outer);
        } else if unsafe { SetWindowRgn(hwnd, None, true) } == 0 {
            self.applied = None;
            return Err("Cannot restore browser window region");
        }
        self.applied = Some(key);
        Ok(())
    }
}

#[cfg(windows)]
struct Region(windows::Win32::Graphics::Gdi::HRGN);

#[cfg(windows)]
impl Region {
    fn new(left: i32, top: i32, right: i32, bottom: i32) -> Result<Self, &'static str> {
        let handle =
            unsafe { windows::Win32::Graphics::Gdi::CreateRectRgn(left, top, right, bottom) };
        if handle.0.is_null() {
            Err("Cannot allocate browser window region")
        } else {
            Ok(Self(handle))
        }
    }
}

#[cfg(windows)]
impl Drop for Region {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Graphics::Gdi::DeleteObject(
                windows::Win32::Graphics::Gdi::HGDIOBJ(self.0.0),
            );
        }
    }
}

/// `None` means no region is needed, including when an overlay misses the window.
/// Invalid/unrepresentable geometry is an error: the caller must hide the native
/// child instead of risking painting or accepting input above a protected overlay.
pub fn cutout(
    browser: eframe::egui::Rect,
    overlay: Option<eframe::egui::Rect>,
    scale: f32,
) -> Result<Option<Cutout>, &'static str> {
    if !browser.is_finite() || !browser.is_positive() || !scale.is_finite() || scale <= 0.0 {
        return Err("Invalid browser clipping geometry");
    }
    // Match Browser::show's f32 multiplication and truncation exactly, then
    // check representability before converting to Win32 integers.
    let x = f64::from((browser.left() * scale).trunc());
    let y = f64::from((browser.top() * scale).trunc());
    let width = f64::from((browser.width() * scale).max(1.0).trunc());
    let height = f64::from((browser.height() * scale).max(1.0).trunc());
    let fits = |value: f64| value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX);
    if width > f64::from(MAX_REGION_COORD) || height > f64::from(MAX_REGION_COORD) {
        return Err("Browser clipping geometry exceeds region limits");
    }
    if ![x, y, width, height, x + width, y + height]
        .into_iter()
        .all(fits)
    {
        return Err("Browser clipping geometry exceeds native limits");
    }
    let Some(overlay) = overlay else {
        return Ok(None);
    };
    if !overlay.is_finite() {
        return Err("Invalid browser overlay geometry");
    }
    if !overlay.is_positive() {
        return Ok(None);
    }
    // Round the occluder outwards: at fractional DPI no native pixels may leak
    // into it. Intersect in physical coordinates to match the actual HWND bounds.
    let left = ((f64::from(overlay.left()) * f64::from(scale)).floor() - x).max(0.0);
    let top = ((f64::from(overlay.top()) * f64::from(scale)).floor() - y).max(0.0);
    let right = ((f64::from(overlay.right()) * f64::from(scale)).ceil() - x).min(width);
    let bottom = ((f64::from(overlay.bottom()) * f64::from(scale)).ceil() - y).min(height);
    if left >= right || top >= bottom {
        return Ok(None);
    }
    Ok(Some(Cutout {
        width: width as i32,
        height: height as i32,
        left: left as i32,
        top: top as i32,
        right: right as i32,
        bottom: bottom as i32,
    }))
}
