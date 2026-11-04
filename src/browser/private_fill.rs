//! Credential evaluation in a separate JavaScript world, never the page's globals.
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use webview2_com::{
    CallDevToolsProtocolMethodCompletedHandler, DevToolsProtocolEventReceivedEventHandler,
    Microsoft::Web::WebView2::Win32::{ICoreWebView2, ICoreWebView2DevToolsProtocolEventReceiver},
    take_pwstr,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};
use wry::WebViewExtWindows;
use zeroize::Zeroizing;

pub struct Reply {
    receiver: mpsc::Receiver<Result<(), String>>,
    live: Arc<AtomicBool>,
}
impl Reply {
    pub fn try_recv(&self) -> Result<Result<(), String>, mpsc::TryRecvError> {
        self.receiver.try_recv()
    }
}
impl Drop for Reply {
    fn drop(&mut self) {
        self.live.store(false, Ordering::Release);
    }
}

fn call(
    view: &ICoreWebView2,
    method: &str,
    params: &str,
    done: impl FnOnce(Result<Value, String>) + 'static,
) -> Result<(), String> {
    let mut wide = Zeroizing::new(Vec::with_capacity(params.len() + 1));
    wide.extend(params.encode_utf16());
    wide.push(0);
    let callback =
        CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |status, response| {
            let result = if status.is_err() || response.len() > 1024 * 1024 {
                Err("Private fill browser operation failed".into())
            } else {
                serde_json::from_str(&response)
                    .map_err(|_| "Invalid private fill browser reply".into())
            };
            done(result);
            Ok(())
        }));
    unsafe {
        view.CallDevToolsProtocolMethod(&HSTRING::from(method), PCWSTR(wide.as_ptr()), &callback)
    }
    .map_err(|_| "Cannot start isolated browser operation".into())
}

struct Fill {
    contexts: Contexts,
    frame: String,
    live: Arc<AtomicBool>,
    view: ICoreWebView2,
    origin: String,
    expression: Zeroizing<String>,
    deadline: Instant,
    sender: mpsc::Sender<Result<(), String>>,
}

const WORLD: &str = "tessel-private-vault-v1";
struct Contexts {
    receiver: ICoreWebView2DevToolsProtocolEventReceiver,
    token: i64,
    view: ICoreWebView2,
    ids: Rc<RefCell<BTreeMap<u64, (String, String)>>>,
}
impl Drop for Contexts {
    fn drop(&mut self) {
        let _ = unsafe {
            self.receiver
                .remove_DevToolsProtocolEventReceived(self.token)
        };
        let _ = call(&self.view, "Runtime.disable", "{}", |_| {});
    }
}
impl Contexts {
    fn subscribe(view: &ICoreWebView2) -> Result<Self, String> {
        let receiver = unsafe {
            view.GetDevToolsProtocolEventReceiver(&HSTRING::from("Runtime.executionContextCreated"))
        }
        .map_err(|_| "Cannot observe private browser contexts")?;
        let ids = Rc::new(RefCell::new(BTreeMap::new()));
        let observed = ids.clone();
        let handler =
            DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                let mut text = PWSTR::null();
                unsafe {
                    args.ParameterObjectAsJson(&mut text)?;
                }
                let text = take_pwstr(text);
                if text.len() > 1024 * 1024 {
                    return Ok(());
                }
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    return Ok(());
                };
                let context = &value["context"];
                if context["name"].as_str() != Some(WORLD)
                    || context["auxData"]["isDefault"].as_bool() != Some(false)
                    || context["auxData"]["type"].as_str() != Some("isolated")
                {
                    return Ok(());
                }
                if let (Some(id), Some(unique), Some(frame)) = (
                    context["id"].as_u64(),
                    context["uniqueId"].as_str(),
                    context["auxData"]["frameId"].as_str(),
                ) && id > 0
                    && !unique.is_empty()
                    && unique.len() <= 256
                    && frame.len() <= 256
                {
                    let mut ids = observed.borrow_mut();
                    if ids.len() < 16 || ids.contains_key(&id) {
                        ids.insert(id, (frame.to_owned(), unique.to_owned()));
                    }
                }
                Ok(())
            }));
        let mut token = 0;
        unsafe { receiver.add_DevToolsProtocolEventReceived(&handler, &mut token) }
            .map_err(|_| "Cannot observe private browser contexts")?;
        Ok(Self {
            receiver,
            token,
            ids,
            view: view.clone(),
        })
    }
}

impl Fill {
    fn check(&self) -> Result<(), String> {
        if !self.live.load(Ordering::Acquire) {
            return Err("Private fill cancelled".into());
        }
        if Instant::now() > self.deadline {
            return Err("Private fill timed out".into());
        }
        let mut source = PWSTR::null();
        unsafe { self.view.Source(&mut source) }.map_err(|_| "Browser is unavailable")?;
        if crate::vault::origin(&take_pwstr(source))? != self.origin {
            return Err("The browser changed site before filling".into());
        }
        Ok(())
    }

    fn request(mut self, method: &str, params: &str, step: u8) {
        if let Err(error) = self.check() {
            let _ = self.sender.send(Err(error));
            return;
        }
        let fallback = self.sender.clone();
        let view = self.view.clone();
        if let Err(error) = call(&view, method, params, move |result| {
            let result = result.and_then(|value| {
                if step != 3 {
                    self.check()?;
                }
                Ok(value)
            });
            let value = match result {
                Ok(value) => value,
                Err(error) => {
                    let error = if step == 3 {
                        "Private fill could not be confirmed; check the login fields.".into()
                    } else {
                        error
                    };
                    let _ = self.sender.send(Err(error));
                    return;
                }
            };
            match step {
                0 => self.request("Page.getFrameTree", "{}", 1),
                1 => {
                    let frame = value.pointer("/frameTree/frame/id").and_then(Value::as_str);
                    let Some(frame) = frame.filter(|id| !id.is_empty() && id.len() <= 256) else {
                        let _ = self
                            .sender
                            .send(Err("Cannot identify the main browser frame".into()));
                        return;
                    };
                    self.frame = frame.to_owned();
                    let params =
                        json!({"frameId":frame,"worldName":WORLD,"grantUniveralAccess":false})
                            .to_string();
                    self.request("Page.createIsolatedWorld", &params, 2);
                }
                2 => {
                    let Some(context) = value
                        .get("executionContextId")
                        .and_then(Value::as_u64)
                        .filter(|id| *id > 0)
                    else {
                        let _ = self
                            .sender
                            .send(Err("Cannot create a private browser world".into()));
                        return;
                    };
                    let unique = self
                        .contexts
                        .ids
                        .borrow()
                        .get(&context)
                        .filter(|(frame, _)| frame == &self.frame)
                        .map(|(_, unique)| unique.clone());
                    let Some(unique) = unique else {
                        let _ = self
                            .sender
                            .send(Err("Cannot verify the private browser context".into()));
                        return;
                    };
                    let unique = serde_json::to_string(&unique).expect("context JSON");
                    let mut encoded =
                        Zeroizing::new(Vec::with_capacity(self.expression.len() * 6 + 2));
                    serde_json::to_writer(&mut *encoded, self.expression.as_str())
                        .expect("string JSON");
                    let mut params =
                        Zeroizing::new(String::with_capacity(encoded.len() + unique.len() + 256));
                    params.push_str("{\"expression\":");
                    params.push_str(std::str::from_utf8(&encoded).expect("JSON UTF8"));
                    params.push_str(",\"uniqueContextId\":");
                    params.push_str(&unique);
                    params.push_str(",\"returnByValue\":true,\"silent\":true,\"includeCommandLineAPI\":false,\"timeout\":3000}");
                    self.request("Runtime.evaluate", &params, 3);
                }
                _ => {
                    // Never expose exception text or arbitrary page-controlled values.
                    let result = if value.get("exceptionDetails").is_some() {
                        Err("Private fill was refused by the browser".into())
                    } else {
                        match value.pointer("/result/value").and_then(Value::as_str) {
                            Some("ok") => Ok(()),
                            Some("wrong-site") => {
                                Err("The browser changed site before filling".into())
                            }
                            Some("unsafe-form") => {
                                Err("The form destination or ownership is unsafe".into())
                            }
                            Some("no-login-form") => {
                                Err("No visible login password field was found".into())
                            }
                            Some("ambiguous-form") => {
                                Err("Choose a page with one unambiguous login form".into())
                            }
                            _ => Err("Private fill did not complete".into()),
                        }
                    };
                    let _ = self.sender.send(result);
                }
            }
        }) {
            let _ = fallback.send(Err(error));
        }
    }
}

pub fn start(
    view: &wry::WebView,
    origin: String,
    expression: Zeroizing<String>,
) -> Result<Reply, String> {
    let view = unsafe { view.controller().CoreWebView2() }.map_err(|_| "Browser unavailable")?;
    let (sender, receiver) = mpsc::channel();
    let live = Arc::new(AtomicBool::new(true));
    let fill = Fill {
        contexts: Contexts::subscribe(&view)?,
        frame: String::new(),
        live: live.clone(),
        view,
        origin,
        expression,
        deadline: Instant::now() + Duration::from_secs(10),
        sender,
    };
    fill.check()?;
    fill.request("Runtime.enable", "{}", 0);
    Ok(Reply { receiver, live })
}
