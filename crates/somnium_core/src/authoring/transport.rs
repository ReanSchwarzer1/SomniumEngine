//! Bounded, current-user local authoring transport. The host polls this on its
//! main thread; transport I/O never owns or mutates the world.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io,
    path::{Path, PathBuf},
};

/// Wire methods accepted by the local editor bridge and MCP adapter.
pub const METHODS: [&str; 7] = [
    "authoring.discover",
    "authoring.query",
    "authoring.plan",
    "authoring.commit",
    "authoring.execute",
    "authoring.history",
    "authoring.jobs",
];
/// Maximum incoming JSON line, excluding its newline.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// Maximum outgoing JSON line, excluding its newline.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Connection descriptor. Its token is a credential; never include it in logs
/// or MCP results. The file is restricted to the current Windows user.
#[derive(Clone, Serialize, Deserialize)]
pub struct ConnectionDescriptor {
    pub version: u32,
    pub project_id: String,
    pub session_id: String,
    pub pipe_name: String,
    pub token: String,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
}

fn failure(id: Value, code: &str, message: &str) -> Value {
    json!({"id": id, "error": {"code": code, "message": message}})
}

fn equal_secret(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .as_bytes()
            .iter()
            .zip(right.as_bytes())
            .fold(0u8, |different, (a, b)| different | (a ^ b))
            == 0
}

/// Validate the transport envelope before the main-thread dispatcher is called.
/// Domain failures returned by dispatch remain in `result`; transport/auth
/// failures have top-level `error`. Domain convention is `{ok:false,error:{...}}`.
pub fn dispatch_envelope(
    descriptor: &ConnectionDescriptor,
    request: Value,
    mut dispatch: impl FnMut(&str, Value) -> Value,
) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if !id.is_string() && !id.is_number() {
        return failure(
            id,
            "invalid_request",
            "A string or numeric request id is required",
        );
    }
    if !request
        .get("token")
        .and_then(Value::as_str)
        .is_some_and(|token| equal_secret(token, &descriptor.token))
    {
        return failure(id, "unauthorized", "The connection credential is invalid");
    }
    if request.get("project_id").and_then(Value::as_str) != Some(descriptor.project_id.as_str()) {
        return failure(
            id,
            "wrong_project",
            "The connection belongs to a different project",
        );
    }
    if request.get("session_id").and_then(Value::as_str) != Some(descriptor.session_id.as_str()) {
        return failure(
            id,
            "session_restarted",
            "The editor session has changed; reconnect and query current state",
        );
    }
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return failure(id, "invalid_request", "A method is required");
    };
    if !METHODS.contains(&method) {
        return failure(
            id,
            "method_not_found",
            "The requested method is not an authoring operation",
        );
    }
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return failure(
            id,
            "invalid_params",
            "Authoring parameters must be an object",
        );
    }
    json!({"id":id, "result":dispatch(method, params)})
}

/// Local bridge owner. On Windows the named pipe rejects remote clients and
/// uses a protected ACL containing only the current user. No worker is spawned.
pub struct LocalBridge {
    descriptor: ConnectionDescriptor,
    descriptor_path: PathBuf,
    #[cfg(windows)]
    pipe: windows::Pipe,
    #[cfg(windows)]
    input: Vec<u8>,
    #[cfg(windows)]
    output: Vec<u8>,
    #[cfg(windows)]
    written: usize,
    #[cfg(windows)]
    last_activity: std::time::Instant,
}

impl LocalBridge {
    /// Bind a new editor session and atomically publish its protected descriptor.
    /// An existing descriptor is replaced; its old pipe remains distinct because
    /// pipe names contain random session IDs. A client must select this descriptor.
    #[cfg(windows)]
    pub fn bind(project_id: &str, descriptor_path: &Path) -> io::Result<Self> {
        use std::io::Write;
        if project_id.is_empty() || project_id.len() > 256 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "project id must contain 1–256 bytes",
            ));
        }
        let session_id = windows::random_hex(16)?;
        let descriptor = ConnectionDescriptor {
            version: 1,
            project_id: project_id.into(),
            pipe_name: format!(r"\\.\pipe\somnium-authoring-{}", session_id),
            session_id,
            token: windows::random_hex(32)?,
            max_request_bytes: MAX_REQUEST_BYTES,
            max_response_bytes: MAX_RESPONSE_BYTES,
        };
        let security = windows::Security::current_user()?;
        let pipe = windows::Pipe::new(&descriptor.pipe_name, &security)?;
        let parent = descriptor_path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "descriptor requires a parent directory",
            )
        })?;
        std::fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(".authoring-{}.tmp", descriptor.session_id));
        let publish = (|| -> io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            // Protect the still-empty file before writing a credential into it.
            security.protect_file(&temporary)?;
            serde_json::to_writer(&mut file, &descriptor)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            drop(file);
            windows::replace_file(&temporary, descriptor_path)
        })();
        if publish.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        publish?;
        Ok(Self {
            descriptor,
            descriptor_path: descriptor_path.into(),
            pipe,
            input: Vec::new(),
            output: Vec::new(),
            written: 0,
            last_activity: std::time::Instant::now(),
        })
    }

    /// Non-Windows hosts explicitly report unsupported transport.
    #[cfg(not(windows))]
    pub fn bind(_project_id: &str, _descriptor_path: &Path) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "local authoring transport currently requires Windows",
        ))
    }

    pub fn descriptor(&self) -> &ConnectionDescriptor {
        &self.descriptor
    }
    pub fn session_id(&self) -> &str {
        &self.descriptor.session_id
    }

    /// Poll without blocking. Processes at most eight requests, 128 KiB input,
    /// and approximately 2 ms per call (dispatch itself must also be bounded).
    /// A disconnected client does not stop the editor; the next client reconnects.
    #[cfg(windows)]
    pub fn poll(&mut self, mut dispatch: impl FnMut(&str, Value) -> Value) -> io::Result<usize> {
        use std::time::{Duration, Instant};
        let was_connected = self.pipe.is_connected();
        if !self.pipe.connect()? {
            return Ok(0);
        }
        if !was_connected {
            self.last_activity = Instant::now();
        }
        if self.last_activity.elapsed() > Duration::from_secs(30) {
            self.reset();
            return Ok(0);
        }
        if !self.output.is_empty() {
            match self.pipe.write(&self.output[self.written..]) {
                Ok(count) => {
                    self.written += count;
                    if count > 0 {
                        self.last_activity = Instant::now();
                    }
                }
                Err(error) if windows::disconnected(&error) => {
                    self.reset();
                    return Ok(0);
                }
                Err(error) => return Err(error),
            }
            if self.written != self.output.len() {
                return Ok(0);
            }
            self.output.clear();
            self.written = 0;
        }
        let started = Instant::now();
        let mut buffer = [0u8; 32 * 1024];
        for _ in 0..4 {
            match self.pipe.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    self.input.extend_from_slice(&buffer[..count]);
                    self.last_activity = Instant::now();
                }
                Err(error) if windows::disconnected(&error) => {
                    self.reset();
                    return Ok(0);
                }
                Err(error) => return Err(error),
            }
            if self.input.len() > MAX_REQUEST_BYTES {
                break;
            }
        }
        if self.input.len() > MAX_REQUEST_BYTES && !self.input.contains(&b'\n') {
            self.reset();
            return Ok(0);
        }
        let mut count = 0;
        while count < 8 && started.elapsed() < Duration::from_millis(2) {
            let Some(end) = self.input.iter().position(|byte| *byte == b'\n') else {
                break;
            };
            let line: Vec<_> = self.input.drain(..=end).collect();
            let response = if end > MAX_REQUEST_BYTES {
                failure(
                    Value::Null,
                    "payload_too_large",
                    "Request exceeds the connection limit",
                )
            } else {
                match serde_json::from_slice::<Value>(&line[..end]) {
                    Ok(request) => dispatch_envelope(&self.descriptor, request, &mut dispatch),
                    Err(_) => failure(Value::Null, "parse_error", "Request is not valid JSON"),
                }
            };
            let mut bytes = serde_json::to_vec(&response)?;
            if bytes.len() > MAX_RESPONSE_BYTES {
                bytes = serde_json::to_vec(&failure(
                    response.get("id").cloned().unwrap_or(Value::Null),
                    "response_too_large",
                    "Query a smaller result page",
                ))?;
            }
            self.output.append(&mut bytes);
            self.output.push(b'\n');
            count += 1;
            // Bound queued response memory to one maximum-sized reply.
            if self.output.len() >= MAX_RESPONSE_BYTES {
                break;
            }
        }
        Ok(count)
    }

    #[cfg(not(windows))]
    pub fn poll(&mut self, _dispatch: impl FnMut(&str, Value) -> Value) -> io::Result<usize> {
        Ok(0)
    }

    #[cfg(windows)]
    fn reset(&mut self) {
        self.pipe.disconnect();
        self.input.clear();
        self.output.clear();
        self.written = 0;
        self.last_activity = std::time::Instant::now();
    }
}

impl Drop for LocalBridge {
    fn drop(&mut self) {
        // Never delete a newer editor's connection descriptor.
        if let Ok(bytes) = std::fs::read(&self.descriptor_path) {
            if serde_json::from_slice::<ConnectionDescriptor>(&bytes)
                .ok()
                .is_some_and(|stored| stored.session_id == self.descriptor.session_id)
            {
                let _ = std::fs::remove_file(&self.descriptor_path);
            }
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, ptr};
    type Handle = *mut c_void;
    const INVALID: Handle = -1isize as Handle;
    #[repr(C)]
    struct SecurityAttributes {
        length: u32,
        descriptor: *mut c_void,
        inherit: i32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateNamedPipeW(
            name: *const u16,
            open: u32,
            mode: u32,
            instances: u32,
            out_size: u32,
            in_size: u32,
            timeout: u32,
            security: *mut SecurityAttributes,
        ) -> Handle;
        fn ConnectNamedPipe(pipe: Handle, overlapped: *mut c_void) -> i32;
        fn DisconnectNamedPipe(pipe: Handle) -> i32;
        fn ReadFile(
            handle: Handle,
            data: *mut c_void,
            size: u32,
            read: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn WriteFile(
            handle: Handle,
            data: *const c_void,
            size: u32,
            written: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
        fn GetCurrentProcess() -> Handle;
        fn LocalFree(memory: Handle) -> Handle;
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
        fn GetTokenInformation(
            token: Handle,
            class: u32,
            data: *mut c_void,
            length: u32,
            needed: *mut u32,
        ) -> i32;
        fn ConvertSidToStringSidW(sid: *mut c_void, text: *mut *mut u16) -> i32;
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text: *const u16,
            revision: u32,
            descriptor: *mut *mut c_void,
            size: *mut u32,
        ) -> i32;
        fn SetFileSecurityW(path: *const u16, information: u32, descriptor: *mut c_void) -> i32;
    }
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(algorithm: Handle, buffer: *mut u8, length: u32, flags: u32) -> i32;
    }

    fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }
    fn checked(ok: i32) -> io::Result<()> {
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    pub fn random_hex(bytes: usize) -> io::Result<String> {
        let mut data = vec![0u8; bytes];
        let result =
            unsafe { BCryptGenRandom(ptr::null_mut(), data.as_mut_ptr(), bytes as u32, 2) };
        if result != 0 {
            return Err(io::Error::other(
                "Windows cryptographic random generation failed",
            ));
        }
        Ok(data.iter().map(|b| format!("{b:02x}")).collect())
    }

    pub struct Security(*mut c_void);
    impl Security {
        pub fn current_user() -> io::Result<Self> {
            unsafe {
                let mut token = ptr::null_mut();
                checked(OpenProcessToken(GetCurrentProcess(), 8, &mut token))?;
                let mut needed = 0;
                GetTokenInformation(token, 1, ptr::null_mut(), 0, &mut needed);
                let mut data = vec![0u8; needed as usize];
                let result = checked(GetTokenInformation(
                    token,
                    1,
                    data.as_mut_ptr().cast(),
                    needed,
                    &mut needed,
                ));
                CloseHandle(token);
                result?;
                if data.len() < std::mem::size_of::<Handle>() {
                    return Err(io::Error::other("Windows token user is missing"));
                }
                let sid = ptr::read_unaligned(data.as_ptr().cast::<Handle>());
                let mut sid_text = ptr::null_mut();
                checked(ConvertSidToStringSidW(sid, &mut sid_text))?;
                let mut length = 0;
                while *sid_text.add(length) != 0 {
                    length += 1;
                }
                let user = String::from_utf16_lossy(std::slice::from_raw_parts(sid_text, length));
                LocalFree(sid_text.cast());
                let sddl = wide(std::ffi::OsStr::new(&format!("D:P(A;;GA;;;{user})")));
                let mut descriptor = ptr::null_mut();
                checked(ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    1,
                    &mut descriptor,
                    ptr::null_mut(),
                ))?;
                Ok(Self(descriptor))
            }
        }
        pub fn protect_file(&self, path: &Path) -> io::Result<()> {
            // DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION.
            unsafe {
                checked(SetFileSecurityW(
                    wide(path.as_os_str()).as_ptr(),
                    0x8000_0004,
                    self.0,
                ))
            }
        }
    }
    impl Drop for Security {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    pub fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
        unsafe {
            checked(MoveFileExW(
                wide(from.as_os_str()).as_ptr(),
                wide(to.as_os_str()).as_ptr(),
                1 | 8,
            ))
        }
    }

    pub struct Pipe {
        handle: Handle,
        connected: bool,
    }
    impl Pipe {
        pub fn is_connected(&self) -> bool {
            self.connected
        }
        pub fn new(name: &str, security: &Security) -> io::Result<Self> {
            let mut attributes = SecurityAttributes {
                length: std::mem::size_of::<SecurityAttributes>() as u32,
                descriptor: security.0,
                inherit: 0,
            };
            let handle = unsafe {
                CreateNamedPipeW(
                    wide(std::ffi::OsStr::new(name)).as_ptr(),
                    3 | 0x0008_0000,
                    1 | 8,
                    1,
                    65536,
                    65536,
                    0,
                    &mut attributes,
                )
            };
            if handle == INVALID {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self {
                    handle,
                    connected: false,
                })
            }
        }
        pub fn connect(&mut self) -> io::Result<bool> {
            if self.connected {
                return Ok(true);
            }
            // With PIPE_NOWAIT, a successful ConnectNamedPipe means the
            // instance became available, not that a client has connected.
            // ERROR_PIPE_CONNECTED on a later poll confirms the connection.
            if unsafe { ConnectNamedPipe(self.handle, ptr::null_mut()) } != 0 {
                return Ok(false);
            }
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(535) => {
                    self.connected = true;
                    Ok(true)
                } // ERROR_PIPE_CONNECTED
                Some(536) => Ok(false), // ERROR_PIPE_LISTENING
                Some(232 | 233 | 109) => {
                    self.disconnect();
                    Ok(false)
                }
                _ => Err(error),
            }
        }
        pub fn read(&self, buffer: &mut [u8]) -> io::Result<usize> {
            let mut read = 0;
            if unsafe {
                ReadFile(
                    self.handle,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut read,
                    ptr::null_mut(),
                )
            } != 0
            {
                return Ok(read as usize);
            }
            let error = io::Error::last_os_error();
            // ERROR_NO_DATA means no bytes currently available for a NOWAIT pipe.
            if matches!(error.raw_os_error(), Some(232 | 536)) {
                Ok(0)
            } else {
                Err(error)
            }
        }
        pub fn write(&self, bytes: &[u8]) -> io::Result<usize> {
            let mut written = 0;
            if unsafe {
                WriteFile(
                    self.handle,
                    bytes.as_ptr().cast(),
                    bytes.len().min(65536) as u32,
                    &mut written,
                    ptr::null_mut(),
                )
            } != 0
            {
                Ok(written as usize)
            } else {
                Err(io::Error::last_os_error())
            }
        }
        pub fn disconnect(&mut self) {
            unsafe {
                DisconnectNamedPipe(self.handle);
            }
            self.connected = false;
        }
    }
    impl Drop for Pipe {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
    pub fn disconnected(error: &io::Error) -> bool {
        matches!(error.raw_os_error(), Some(109 | 232 | 233))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn descriptor() -> ConnectionDescriptor {
        ConnectionDescriptor {
            version: 1,
            project_id: "p".into(),
            session_id: "s".into(),
            pipe_name: "unused".into(),
            token: "secret".into(),
            max_request_bytes: MAX_REQUEST_BYTES,
            max_response_bytes: MAX_RESPONSE_BYTES,
        }
    }
    #[test]
    fn authentication_and_native_allowlist_precede_dispatch() {
        let d = descriptor();
        for (token, project, session, method, expected) in [
            ("bad", "p", "s", "authoring.query", "unauthorized"),
            ("secret", "other", "s", "authoring.query", "wrong_project"),
            ("secret", "p", "old", "authoring.query", "session_restarted"),
            ("secret", "p", "s", "shell", "method_not_found"),
        ] {
            let reply = dispatch_envelope(
                &d,
                json!({"id":1,"token":token,"project_id":project,"session_id":session,"method":method,"params":{}}),
                |_, _| panic!("invalid envelope reached dispatcher"),
            );
            assert_eq!(reply["error"]["code"], expected);
        }
    }
    #[test]
    fn only_validated_params_reach_dispatch_and_result_is_preserved() {
        let d = descriptor();
        let response = dispatch_envelope(
            &d,
            json!({"id":"request-1","token":"secret","project_id":"p","session_id":"s","method":"authoring.query","params":{"revision":3}}),
            |method, params| {
                assert_eq!(method, "authoring.query");
                assert_eq!(params["revision"], 3);
                json!({"ok":false,"error":{"code":"revision_conflict"}})
            },
        );
        assert_eq!(response["id"], "request-1");
        assert_eq!(response["result"]["error"]["code"], "revision_conflict");
    }
}
