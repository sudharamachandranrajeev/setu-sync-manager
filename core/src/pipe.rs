//! The local channel between `setu` (command line) and the running Setu, which hosts the sync
//! engine. One JSON request per line, one JSON reply per line:
//! `{"method": "...", "args": {...}}` → `{"ok": ...}` or `{"err": "..."}`.
//!
//! Only this user can use it. Windows: a named pipe named after the user's SID, whose access
//! list admits only that user, refusing remote clients; the command line also checks that the
//! pipe it opened belongs to the same user. Linux: a socket with mode 0600 in
//! `$XDG_RUNTIME_DIR`, which is the user's own.
//!
//! The pipe doubles as Setu's single-instance lock: a second Setu finds it taken and asks the
//! first one to show its window instead.

use std::io::{BufRead, BufReader, Write};
use std::sync::Arc;

use serde_json::{Value, json};

use crate::Error;
use crate::syncthing::Transport;

/// What the running Setu does with a request.
pub type Handler = Arc<dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync>;

const NOT_RUNNING: &str = "Setu is not running. Start Setu (or `setu-app --background`) and try again.";

/// The command line's end: one connection per call.
pub struct Pipe;

impl Pipe {
    /// Check the running Setu answers.
    pub fn connect() -> Result<Pipe, Error> {
        Pipe.call("ping", &Value::Null)?;
        Ok(Pipe)
    }
}

impl Transport for Pipe {
    fn call(&self, method: &str, args: &Value) -> Result<Value, Error> {
        let stream = sys::open()?;
        let mut w = &stream;
        let mut line = json!({ "method": method, "args": args }).to_string();
        line.push('\n');
        w.write_all(line.as_bytes()).and_then(|()| w.flush()).map_err(|e| Error::Setup(format!("talking to Setu: {e}")))?;
        let mut reply = String::new();
        BufReader::new(&stream).read_line(&mut reply).map_err(|e| Error::Setup(format!("talking to Setu: {e}")))?;
        if reply.is_empty() {
            return Err(Error::Setup("Setu closed the connection".into()));
        }
        let mut v: Value = serde_json::from_str(&reply).map_err(|e| Error::Setup(format!("Setu's reply: {e}")))?;
        match v.get_mut("err").map(Value::take) {
            Some(Value::String(e)) => Err(Error::Syncthing(e)),
            _ => Ok(v.get_mut("ok").map(Value::take).unwrap_or(Value::Null)),
        }
    }
}

/// Ask the running Setu to quit and wait up to 15 s until it is gone. `Ok(false)`: none ran.
pub fn quit_running() -> Result<bool, Error> {
    if Pipe::connect().is_err() {
        return Ok(false);
    }
    let _ = Pipe.call("quit", &Value::Null);
    for _ in 0..150 {
        if Pipe::connect().is_err() {
            return Ok(true);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err(Error::Setup("Setu did not quit within 15 s".into()))
}

/// The running Setu's end, bound but not yet answering.
pub struct Listener(sys::Listener);

/// Take the pipe. `None` when another Setu already has it.
pub fn listen() -> Result<Option<Listener>, Error> {
    Ok(sys::listen()?.map(Listener))
}

impl Listener {
    /// Answer requests on a background thread, each connection on its own thread (an `events`
    /// call can wait a minute).
    pub fn serve(self, handler: Handler) {
        std::thread::spawn(move || self.0.serve(&handler));
    }
}

/// Read requests from one connection until it closes, answering each. Public so the end-to-end
/// tests can serve the same protocol from engine host processes over loopback.
pub fn answer(r: impl std::io::Read, mut w: impl Write, handler: &Handler) {
    let mut reader = BufReader::new(r);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(req) => {
                let method = req["method"].as_str().unwrap_or_default();
                match method {
                    "ping" => Ok(json!("pong")),
                    _ => handler(method, &req["args"]),
                }
            }
            Err(e) => Err(format!("bad request: {e}")),
        };
        let mut out = match reply {
            Ok(v) => json!({ "ok": v }),
            Err(e) => json!({ "err": e }),
        }
        .to_string();
        out.push('\n');
        if w.write_all(out.as_bytes()).and_then(|()| w.flush()).is_err() {
            return;
        }
    }
}

#[cfg(unix)]
mod sys {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;

    use super::{Handler, NOT_RUNNING, answer};
    use crate::Error;

    fn path() -> PathBuf {
        if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|d| d.is_dir()) {
            return d.join("setu.sock");
        }
        crate::local::config_dir().join("setu.sock")
    }

    pub fn open() -> Result<UnixStream, Error> {
        UnixStream::connect(path()).map_err(|_| Error::Setup(NOT_RUNNING.into()))
    }

    pub struct Listener(UnixListener);

    pub fn listen() -> Result<Option<Listener>, Error> {
        let p = path();
        if p.exists() {
            if UnixStream::connect(&p).is_ok() {
                return Ok(None);
            }
            // Left behind by a Setu that did not exit cleanly.
            std::fs::remove_file(&p)?;
        }
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let l = UnixListener::bind(&p)?;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
        Ok(Some(Listener(l)))
    }

    impl Listener {
        pub fn serve(self, handler: &Handler) {
            for s in self.0.incoming().flatten() {
                let h = handler.clone();
                std::thread::spawn(move || answer(&s, &s, &h));
            }
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;
    use std::fs::File;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1, SE_KERNEL_OBJECT,
    };
    use windows_sys::Win32::Security::{
        EqualSid, GetTokenInformation, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    };
    use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    use super::{Handler, NOT_RUNNING, answer};
    use crate::Error;

    /// This user's SID: the bytes of the TOKEN_USER (which points into itself) and its text.
    struct User {
        token_user: Vec<u64>,
        text: String,
    }

    impl User {
        fn sid(&self) -> PSID {
            // SAFETY: token_user holds a TOKEN_USER written by GetTokenInformation.
            unsafe { (*(self.token_user.as_ptr() as *const TOKEN_USER)).User.Sid }
        }
    }

    fn user() -> Result<User, Error> {
        let fail = |what: &str| Error::Setup(format!("{what}: {}", std::io::Error::last_os_error()));
        // SAFETY: plain Win32 calls with buffers sized as the API reports.
        unsafe {
            let mut token: HANDLE = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(fail("reading this user"));
            }
            let mut len = 0u32;
            GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len);
            let mut buf = vec![0u64; (len as usize).div_ceil(8)];
            let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len);
            CloseHandle(token);
            if ok == 0 {
                return Err(fail("reading this user"));
            }
            let mut u = User { token_user: buf, text: String::new() };
            let mut s: *mut u16 = null_mut();
            if ConvertSidToStringSidW(u.sid(), &mut s) == 0 {
                return Err(fail("reading this user"));
            }
            let n = (0..).take_while(|&i| *s.add(i) != 0).count();
            u.text = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
            LocalFree(s.cast());
            Ok(u)
        }
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn name(u: &User) -> String {
        format!(r"\\.\pipe\setu-{}", u.text)
    }

    pub fn open() -> Result<File, Error> {
        let u = user()?;
        let path = name(&u);
        for _ in 0..50 {
            match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
                Ok(f) => {
                    if !owned_by(&f, &u) {
                        return Err(Error::Setup("the Setu pipe belongs to another user; not using it".into()));
                    }
                    return Ok(f);
                }
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => std::thread::sleep(std::time::Duration::from_millis(100)),
                Err(_) => break,
            }
        }
        Err(Error::Setup(NOT_RUNNING.into()))
    }

    /// Is the pipe's owner this user? Guards against another user's program taking the name first.
    fn owned_by(f: &File, u: &User) -> bool {
        let mut owner: PSID = null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: owner points into sd, which is freed after the comparison.
        unsafe {
            let r = GetSecurityInfo(
                f.as_raw_handle() as HANDLE,
                SE_KERNEL_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                null_mut(),
                null_mut(),
                &mut sd,
            );
            let same = r == 0 && !owner.is_null() && EqualSid(owner, u.sid()) != 0;
            if !sd.is_null() {
                LocalFree(sd);
            }
            same
        }
    }

    pub struct Listener {
        user: User,
        first: HANDLE,
    }

    // The handle is only used by the serving thread.
    unsafe impl Send for Listener {}

    fn create(u: &User, first: bool) -> Result<HANDLE, u32> {
        // Owner = this user; only this user gets access; nobody else, not even administrators.
        let sddl = wide(&format!("O:{0}D:P(A;;GA;;;{0})", u.text));
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: the security descriptor lives until CreateNamedPipeW has copied it.
        unsafe {
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, null_mut()) == 0 {
                return Err(GetLastError());
            }
            let sa = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
            let open_mode = PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
            let h = CreateNamedPipeW(
                wide(&name(u)).as_ptr(),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                &sa as *const SECURITY_ATTRIBUTES as *const c_void as *const SECURITY_ATTRIBUTES,
            );
            let err = GetLastError();
            LocalFree(sd);
            if h == INVALID_HANDLE_VALUE { Err(err) } else { Ok(h) }
        }
    }

    pub fn listen() -> Result<Option<Listener>, Error> {
        let user = user()?;
        match create(&user, true) {
            Ok(first) => Ok(Some(Listener { user, first })),
            Err(ERROR_ACCESS_DENIED) => Ok(None),
            Err(e) => Err(Error::Setup(format!("Setu's pipe: {}", std::io::Error::from_raw_os_error(e as i32)))),
        }
    }

    impl Listener {
        pub fn serve(self, handler: &Handler) {
            let mut h = self.first;
            loop {
                // SAFETY: h is a pipe instance created above and not yet connected.
                let connected = unsafe { ConnectNamedPipe(h, null_mut()) } != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
                if connected {
                    // SAFETY: the File takes ownership of the handle and closes it when done.
                    let f = unsafe { File::from_raw_handle(h as _) };
                    let hd = handler.clone();
                    std::thread::spawn(move || answer(&f, &f, &hd));
                } else {
                    unsafe { CloseHandle(h) };
                }
                h = match create(&self.user, false) {
                    Ok(h) => h,
                    Err(e) => {
                        eprintln!("setu: pipe: {}", std::io::Error::from_raw_os_error(e as i32));
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        continue;
                    }
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_one_line_per_request() {
        let handler: Handler = Arc::new(|m, a| if m == "echo" { Ok(a.clone()) } else { Err(format!("no {m}")) });
        let input = b"{\"method\":\"echo\",\"args\":{\"x\":1}}\n{\"method\":\"ping\"}\nnot json\n{\"method\":\"nope\"}\n".to_vec();
        let mut out = Vec::new();
        answer(&input[..], &mut out, &handler);
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines[0], json!({"ok": {"x": 1}}));
        assert_eq!(lines[1], json!({"ok": "pong"}));
        assert!(lines[2]["err"].as_str().unwrap().starts_with("bad request"));
        assert_eq!(lines[3], json!({"err": "no nope"}));
    }

    /// The real pipe (Windows) or socket (Linux): calls arrive, errors come back as errors, and
    /// a second Setu finds the pipe taken. Skipped while a real Setu runs for this user.
    #[test]
    fn real_pipe_round_trip_and_single_instance() {
        let Some(l) = listen().unwrap() else {
            eprintln!("Setu is running; skipping");
            return;
        };
        l.serve(Arc::new(|m, a| if m == "echo" { Ok(a.clone()) } else { Err(format!("no {m}")) }));
        assert!(listen().unwrap().is_none(), "a second listener must find the pipe taken");
        let p = Pipe::connect().unwrap();
        let threads: Vec<_> = (0..4)
            .map(|i| std::thread::spawn(move || Pipe.call("echo", &json!({ "n": i })).unwrap()))
            .collect();
        for (i, t) in threads.into_iter().enumerate() {
            assert_eq!(t.join().unwrap(), json!({ "n": i }));
        }
        assert!(matches!(p.call("nope", &Value::Null), Err(Error::Syncthing(e)) if e == "no nope"));
    }
}
