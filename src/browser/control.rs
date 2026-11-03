//! Narrow DOM operations in an explicitly enabled, project-specific browser.
use super::*;
use serde_json::{Value, json};
use std::{
    path::{Component, Path},
    time::{Duration, Instant},
};
pub type ControlReply = Receiver<Result<Value, String>>;

fn folder_in(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
        || relative.contains(':')
    {
        return Err("Download folder must be relative to this project".into());
    }
    let path = root
        .join(path)
        .canonicalize()
        .map_err(|_| "Choose an existing project folder")?;
    if !path.starts_with(root) || !path.is_dir() {
        return Err("Download folder leaves this project".into());
    }
    Ok(path)
}
fn script(args: &Value) -> Result<String, String> {
    let op = args["operation"].as_str().ok_or("Missing operation")?;
    if !matches!(
        op,
        "snapshot" | "click" | "fill" | "select" | "check" | "scroll"
    ) {
        return Err("Unsupported DOM operation".into());
    }
    if op != "snapshot" {
        let origin = args["origin"]
            .as_str()
            .ok_or("Use the exact origin returned by snapshot")?;
        let url = url::Url::parse(origin).map_err(|_| "Invalid origin")?;
        if !matches!(url.scheme(), "http" | "https") || url.origin().ascii_serialization() != origin
        {
            return Err("Invalid HTTP(S) origin".into());
        }
        let selector = args["selector"].as_str().ok_or("Missing selector")?;
        if selector.is_empty() || selector.len() > 2048 {
            return Err("Selector must be 1 to 2048 bytes".into());
        }
        if op == "fill" && args["text"].as_str().is_none_or(|s| s.len() > 16384) {
            return Err("Fill text is limited to 16 KiB".into());
        }
    }
    if op == "select" && args["value"].as_str().is_none_or(|v| v.len() > 4096) {
        return Err("Select value is limited to 4096 bytes".into());
    }
    if op == "check" && !args["checked"].is_boolean() {
        return Err("Checked must be a boolean".into());
    }
    let payload = serde_json::to_string(args).map_err(|_| "Invalid arguments")?;
    Ok(format!(
        r#"(() => {{ try {{
const a={payload};
const visible=e=>!!e.getClientRects().length && getComputedStyle(e).visibility!=='hidden';
if(a.operation==='snapshot') {{
 let remainingOptions=64;const options=e=>{{const values=[...e.options].slice(0,Math.min(20,remainingOptions));remainingOptions-=values.length;return values.map(o=>({{value:o.value.slice(0,4096),label:o.label.slice(0,256),disabled:o.disabled||!!o.closest('optgroup')?.disabled}}));}};
 const selector=e=>{{ if(e.id && document.querySelectorAll('#'+CSS.escape(e.id)).length===1) return '#'+CSS.escape(e.id); let parts=[]; for(let n=e;n&&n.nodeType===1;n=n.parentElement) {{ let s=n.tagName.toLowerCase(), i=1;for(let p=n.previousElementSibling;p;p=p.previousElementSibling)if(p.tagName===n.tagName)i++;parts.unshift(s+':nth-of-type('+i+')');if(n===document.documentElement)break; }} return parts.join(' > '); }};
 return {{url:location.href,origin:location.origin,title:document.title.slice(0,512),text:(document.body?.innerText||'').slice(0,48000),elements:[...document.querySelectorAll('a,button,input,textarea,select,[role=button]')].filter(visible).slice(0,150).map(e=>({{selector:selector(e),tag:e.tagName.toLowerCase(),type:e.type||'',label:(e.getAttribute('aria-label')||e.innerText||e.placeholder||'').slice(0,256),disabled:!!e.disabled,checked:['checkbox','radio'].includes(e.type)?e.checked:undefined,options:e.tagName==='SELECT'?options(e):undefined}})),scroll:{{x:scrollX,y:scrollY,width:innerWidth,height:innerHeight}},untrusted_page_content:true}};
}}
if(location.origin!==a.origin) return {{error:'Page origin changed; take a new snapshot'}};
const nodes=document.querySelectorAll(a.selector);
if(nodes.length!==1) return {{error:'Selector must match exactly one element'}};
const e=nodes[0];
if(!visible(e)||e.disabled) return {{error:'Element is hidden or disabled'}};
if(e.tagName==='INPUT'&&['file','password'].includes(e.type)) return {{error:'File and password fields require the owner'}};
if(a.operation==='scroll') {{ e.scrollIntoView({{block:'center',inline:'nearest',behavior:'instant'}});return {{performed:true}}; }}
if(a.operation==='select') {{
 if(e.tagName!=='SELECT'||e.multiple)return {{error:'Select requires one single-choice dropdown'}};
 const opts=[...e.options].filter(o=>o.value===a.value);
 if(opts.length!==1||opts[0].disabled||opts[0].closest('optgroup')?.disabled)return {{error:'Option is missing, ambiguous or disabled'}};
 Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'value').set.call(e,a.value);
 e.dispatchEvent(new Event('input',{{bubbles:true}}));e.dispatchEvent(new Event('change',{{bubbles:true}}));return {{performed:true}};
}}
if(a.operation==='check') {{
 if(e.tagName!=='INPUT'||!['checkbox','radio'].includes(e.type)||e.type==='radio'&&!a.checked)return {{error:'Check requires a checkbox or selecting a radio button'}};
 if(e.checked!==a.checked)e.click();return {{performed:e.checked===a.checked}};
}}
if(a.operation==='click') {{ e.click();return {{performed:true}}; }}
if(!['INPUT','TEXTAREA'].includes(e.tagName)||e.readOnly) return {{error:'Element is not an editable text field'}};
if(e.tagName==='INPUT'&&!['text','email','search','tel','url','number'].includes(e.type)) return {{error:'Unsupported input type'}};
const proto=e.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype;
Object.getOwnPropertyDescriptor(proto,'value').set.call(e,a.text);e.dispatchEvent(new Event('input',{{bubbles:true}}));e.dispatchEvent(new Event('change',{{bubbles:true}}));return {{performed:true}};
}}catch(_){{return {{error:'DOM operation failed; take a fresh snapshot'}};}} }})()"#
    ))
}
impl Browser {
    pub fn start_agent(frame: &eframe::Frame, root: &Path) -> Result<Self, String> {
        let root = root
            .canonicalize()
            .map_err(|_| "Project folder unavailable")?;
        let profile = tempfile::Builder::new()
            .prefix("tessel-agent-browser-")
            .tempdir()
            .map_err(|_| "Cannot create isolated browser profile")?;
        let mut browser = Self {
            agent_root: Some(root),
            address: "about:blank".into(),
            ..Default::default()
        };
        browser.create_with_profile(frame, profile.path().to_owned())?;
        browser.agent_profile = Some(profile);
        Ok(browser)
    }
    pub fn control(&mut self, args: &Value) -> Result<ControlReply, String> {
        self.poll();
        let root = self
            .agent_root
            .as_ref()
            .ok_or("Owner browser is never available to agent tools")?;
        let obj = args.as_object().ok_or("Arguments must be an object")?;
        let operation = args["operation"].as_str().ok_or("Missing operation")?;
        let keys: &[&str] = match operation {
            "snapshot" | "downloads" => &["operation"],
            "navigate" => &["operation", "url"],
            "click" | "scroll" => &["operation", "origin", "selector"],
            "select" => &["operation", "origin", "selector", "value"],
            "check" => &["operation", "origin", "selector", "checked"],
            "fill" => &["operation", "origin", "selector", "text"],
            "prepare_download" => &["operation", "folder", "filename"],
            _ => return Err("Unknown browser operation".into()),
        };
        if obj.keys().any(|k| !keys.contains(&k.as_str())) {
            return Err("Unexpected browser argument".into());
        }
        let (tx, rx) = mpsc::channel();
        let value = match operation {
            "downloads" => {
                json!({"downloads":self.downloads.iter().map(|(p,s)|json!({"path":p,"status":s})).collect::<Vec<_>>(),"error":self.error})
            }
            "prepare_download" => {
                let folder = folder_in(root, args["folder"].as_str().unwrap_or("."))?;
                let name = args["filename"].as_str().ok_or("Missing filename")?;
                if name.is_empty() || safe_name(name) != name {
                    return Err(
                        "Filename must be a safe single filename, at most 160 characters".into(),
                    );
                }
                self.error.clear();
                let mut plan = self
                    .planned_download
                    .lock()
                    .map_err(|_| "Download settings unavailable")?;
                if plan
                    .as_ref()
                    .is_some_and(|(_, deadline)| Instant::now() < *deadline)
                {
                    return Err("A download is already prepared; trigger it or wait for its 60-second expiry".into());
                }
                let path = download_target(&folder, Path::new(name))?;
                *plan = Some((path.clone(), Instant::now() + Duration::from_secs(60)));
                json!({"path":path,"expires_in_seconds":60,"instruction":"Trigger one download in this browser. A unique subfolder prevents overwriting. Poll downloads for completion."})
            }
            "navigate" => {
                let address = args["url"]
                    .as_str()
                    .filter(|u| u.len() <= 8192)
                    .ok_or("Missing or oversized URL")?;
                let url = url::Url::parse(address).map_err(|_| "Invalid URL")?;
                if !allowed(address) || !url.username().is_empty() || url.password().is_some() {
                    return Err("Unsupported navigation URL".into());
                }
                self.view
                    .as_ref()
                    .ok_or("Browser unavailable")?
                    .load_url(address)
                    .map_err(|_| "Navigation failed")?;
                json!({"navigation_started":true,"instruction":"Take a fresh snapshot after the page loads."})
            }
            _ => {
                let js = script(args)?;
                self.view
                    .as_ref()
                    .ok_or("Browser unavailable")?
                    .evaluate_script_with_callback(&js, move |text| {
                        let result = if text.len() > 512 * 1024 {
                            Err("Browser result exceeded 512 KiB".into())
                        } else {
                            serde_json::from_str::<Value>(&text)
                                .map_err(|_| "Invalid browser result".to_string())
                                .and_then(|v| {
                                    if let Some(e) = v["error"].as_str() {
                                        Err(e.into())
                                    } else if v.is_object() {
                                        Ok(v)
                                    } else {
                                        Err("Page navigation interrupted the browser operation"
                                            .into())
                                    }
                                })
                        };
                        let _ = tx.send(result);
                    })
                    .map_err(|_| "Browser operation failed")?;
                return Ok(rx);
            }
        };
        let _ = tx.send(Ok(value));
        Ok(rx)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn download_folder_is_scoped_and_dom_requires_origin() {
        assert!(script(&json!({"operation":"check","origin":"https://example.org","selector":"#checkbox","checked":"true"})).is_err());
        assert!(
            script(
                &json!({"operation":"select","origin":"https://example.org","selector":"#dropdown"})
            )
            .is_err()
        );
        assert!(
            script(
                &json!({"operation":"scroll","origin":"https://example.org","selector":"#target"})
            )
            .is_ok()
        );
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        assert!(folder_in(&root, ".").is_ok());
        for p in ["../escape", "C:/Windows", "/outside", "a:stream"] {
            assert!(folder_in(&root, p).is_err());
        }
        assert!(script(&json!({"operation":"click","selector":"button"})).is_err());
        assert!(
            script(&json!({"operation":"click","origin":"file:///C:/","selector":"button"}))
                .is_err()
        );
        assert!(script(&json!({"operation":"fill","origin":"https://example.org","selector":"#query","text":"quote'\";alert(1)"})).is_ok());
    }
    #[test]
    fn owner_browser_refuses_agent_requests() {
        let mut owner = Browser::default();
        assert!(owner.control(&json!({"operation":"downloads"})).is_err());
    }
}
