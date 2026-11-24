//! Syncthing's code, built into Setu (see `go/engine.go`). One engine per process: `start` it
//! once, make `call`s, `stop` it before the process ends.
//!
//! Every call takes and returns JSON in the shapes of Syncthing's REST API, but nothing goes
//! over a network or a port: these are plain function calls into the linked Go code.

use std::ffi::{CStr, CString, c_char};
use std::path::PathBuf;
use std::sync::Once;

use serde::Serialize;
use serde_json::Value;

unsafe extern "C" {
    fn setu_engine_start(args: *const c_char) -> *mut c_char;
    fn setu_engine_stop() -> *mut c_char;
    fn setu_engine_call(method: *const c_char, args: *const c_char) -> *mut c_char;
    fn setu_engine_free(p: *mut c_char);
}

#[cfg(windows)]
unsafe extern "C" {
    /// The Go runtime's start-up for a library. MinGW runs it from `.ctors`, which the MSVC
    /// C runtime does not, so on Windows Setu runs it before the first call.
    fn _rt0_amd64_windows_lib();
}

fn init_runtime() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        #[cfg(windows)]
        // SAFETY: called once, before any other call into the Go code.
        unsafe {
            _rt0_amd64_windows_lib()
        };
    });
}

#[derive(Serialize)]
pub struct StartArgs {
    /// Keys, config and database.
    pub home: PathBuf,
    /// Addresses to listen on; Setu passes only Tailscale addresses.
    pub listen: Vec<String>,
    /// This computer's name, used when the engine's config is new.
    pub name: String,
}

/// Start the engine. Returns this computer's device ID.
pub fn start(args: &StartArgs) -> Result<String, String> {
    let json = serde_json::to_string(args).map_err(|e| e.to_string())?;
    let v = take(|| unsafe { setu_engine_start(cstr(&json).as_ptr()) })?;
    v["myID"].as_str().map(String::from).ok_or_else(|| "engine started without a device ID".into())
}

/// Stop syncing, close the database and wait until everything has stopped.
pub fn stop() -> Result<(), String> {
    take(|| unsafe { setu_engine_stop() }).map(drop)
}

/// One call into the engine, e.g. `call("folder_status", &json!({"folder": "setu-f-…"}))`.
pub fn call(method: &str, args: &Value) -> Result<Value, String> {
    let args = if args.is_null() { String::new() } else { args.to_string() };
    take(|| unsafe { setu_engine_call(cstr(method).as_ptr(), cstr(&args).as_ptr()) })
}

/// Runs an export, reads its `{"ok": …}` or `{"err": …}` reply and frees it.
fn take(f: impl FnOnce() -> *mut c_char) -> Result<Value, String> {
    init_runtime();
    let p = f();
    if p.is_null() {
        return Err("no reply from the engine".into());
    }
    // SAFETY: the engine returns a NUL-terminated string it allocated; it is freed once, here.
    let text = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    unsafe { setu_engine_free(p) };
    let mut v: Value = serde_json::from_str(&text).map_err(|e| format!("engine reply: {e}"))?;
    match v.get_mut("err").map(Value::take) {
        Some(Value::String(e)) => Err(e),
        _ => Ok(v.get_mut("ok").map(Value::take).unwrap_or(Value::Null)),
    }
}

fn cstr(s: &str) -> CString {
    // Interior NULs cannot come from JSON text or method names; drop them rather than fail.
    CString::new(s.replace('\0', "")).unwrap()
}
