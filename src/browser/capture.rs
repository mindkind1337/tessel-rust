//! Explicit project-agent screenshots; original engine PNG, no desktop capture.
use super::control::ControlReply;
use serde_json::json;
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use webview2_com::{
    CapturePreviewCompletedHandler, Microsoft::Web::WebView2::Win32::*, take_pwstr,
};
use windows::{
    Win32::{
        Foundation::{HGLOBAL, RECT},
        System::Com::{
            STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET, StructuredStorage::CreateStreamOnHGlobal,
        },
    },
    core::PWSTR,
};
use wry::WebViewExtWindows;
pub(super) struct Capture {
    live: Arc<AtomicBool>,
    pending: Arc<AtomicBool>,
}
impl Default for Capture {
    fn default() -> Self {
        Self {
            live: Arc::new(AtomicBool::new(true)),
            pending: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.live.store(false, Ordering::Release);
    }
}
struct Pending(Arc<AtomicBool>);
impl Drop for Pending {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
fn origin(view: &ICoreWebView2) -> Result<String, String> {
    let mut source = PWSTR::null();
    unsafe { view.Source(&mut source) }.map_err(|_| "Cannot read browser origin")?;
    let url = url::Url::parse(&take_pwstr(source)).map_err(|_| "Invalid browser origin")?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Capture requires an HTTP(S) page".into());
    }
    Ok(url.origin().ascii_serialization())
}
fn png_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return Err("Browser did not return a PNG".into());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return Err("Capture dimensions exceed 4096 pixels per side".into());
    }
    Ok((width, height))
}
impl Capture {
    pub fn start(
        &self,
        view: &wry::WebView,
        root: PathBuf,
        path: PathBuf,
        expected: String,
    ) -> Result<ControlReply, String> {
        if self.pending.swap(true, Ordering::AcqRel) {
            return Err("A screenshot is already pending".into());
        }
        let guard = Pending(self.pending.clone());
        let controller = view.controller();
        let webview = unsafe { controller.CoreWebView2() }.map_err(|_| "Browser unavailable")?;
        if origin(&webview)? != expected {
            return Err("Page origin changed; take a new snapshot".into());
        }
        let mut rect = RECT::default();
        unsafe { controller.Bounds(&mut rect) }.map_err(|_| "Cannot read browser dimensions")?;
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        if !(1..=4096).contains(&width) || !(1..=4096).contains(&height) {
            return Err("Browser viewport must be at most 4096 pixels per side".into());
        }
        let stream = unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true) }
            .map_err(|_| "Cannot allocate capture stream")?;
        let result_stream = stream.clone();
        let result_view = webview.clone();
        let live = self.live.clone();
        let deadline = Instant::now() + Duration::from_secs(20);
        let (tx, rx) = mpsc::channel();
        unsafe{webview.CapturePreview(COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,&stream,&CapturePreviewCompletedHandler::create(Box::new(move|status|{
            let result=(||->Result<Vec<u8>,String>{
                status.map_err(|_|"Browser capture failed")?;
                if !live.load(Ordering::Acquire)||Instant::now()>deadline{return Err("Capture cancelled or timed out".into());}
                if origin(&result_view)?!=expected{return Err("Page changed during capture; take a new snapshot".into());}
                let mut stat=STATSTG::default();result_stream.Stat(&mut stat,STATFLAG_NONAME).map_err(|_|"Cannot read capture size")?;
                if stat.cbSize>32*1024*1024{return Err("Screenshot exceeds 32 MiB".into());}
                result_stream.Seek(0,STREAM_SEEK_SET,None).map_err(|_|"Cannot read capture stream")?;
                let mut bytes=vec![0;stat.cbSize as usize];let mut read=0;
                result_stream.Read(bytes.as_mut_ptr().cast(),bytes.len() as u32,Some(&mut read)).ok().map_err(|_|"Cannot read capture")?;
                if read as usize!=bytes.len(){return Err("Incomplete capture".into());}png_dimensions(&bytes)?;Ok(bytes)
            })();
            match result{
                Err(error)=>{let _=tx.send(Err(error));drop(guard);}
                Ok(bytes)=>{std::thread::spawn(move||{
                    let _guard=guard;
                    let result=(||->Result<serde_json::Value,String>{
                        if !live.load(Ordering::Acquire)||Instant::now()>deadline{return Err("Capture cancelled or timed out".into());}
                        let parent=path.parent().ok_or("Missing capture folder")?;
                        let canonical=parent.canonicalize().map_err(|_|"Capture folder unavailable")?;
                        if !canonical.starts_with(&root)||canonical!=parent{return Err("Capture folder changed or left this project".into());}
                        let(width,height)=png_dimensions(&bytes)?;
                        let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|_|"Cannot create screenshot; destination exists or is unavailable")?;
                        if file.write_all(&bytes).and_then(|_|file.sync_all()).is_err(){drop(file);let _=std::fs::remove_file(&path);return Err("Cannot save screenshot".into());}
                        Ok(json!({"path":path,"mime_type":"image/png","width":width,"height":height,"bytes":bytes.len(),"untrusted_page_content":true}))
                    })();let _=tx.send(result);
                });}
            }
            Ok(())
        })))}.map_err(|_|"Cannot start browser capture")?;
        Ok(rx)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screenshot_dimensions_are_bounded() {
        assert!(png_dimensions(b"broken").is_err());
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend(4096u32.to_be_bytes());
        b.extend(200u32.to_be_bytes());
        assert_eq!(png_dimensions(&b).unwrap(), (4096, 200));
        b[16..20].copy_from_slice(&4097u32.to_be_bytes());
        assert!(png_dimensions(&b).is_err());
    }
}
