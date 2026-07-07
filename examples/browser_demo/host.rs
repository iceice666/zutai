#[cfg(unix)]
mod unix_host {
    use std::ffi::{CStr, CString, c_char, c_void};
    use std::io::{self, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::slice;

    const INDEX_HTML: &str = include_str!("index.html");
    const DEFAULT_ADDR: &str = "127.0.0.1:8787";
    const RTLD_NOW: i32 = 2;
    const RTLD_LOCAL: i32 = 0;

    type EntryJsonFn = unsafe extern "C" fn() -> i64;
    type TextAbiFn = unsafe extern "C" fn(i64) -> i64;

    #[cfg_attr(target_os = "linux", link(name = "dl"))]
    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flags: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlerror() -> *mut c_char;
        fn dlclose(handle: *mut c_void) -> i32;
    }

    pub fn run() -> Result<(), String> {
        let mut args = std::env::args().skip(1);
        let lib_path = args
            .next()
            .ok_or_else(|| "usage: host <compiled-zutai-library> [addr]".to_string())?;
        let addr = args.next().unwrap_or_else(|| DEFAULT_ADDR.to_string());
        let library = ZutaiLibrary::open(Path::new(&lib_path))?;
        let listener =
            TcpListener::bind(&addr).map_err(|err| format!("failed to bind {addr}: {err}"))?;

        println!("Zutai browser demo serving http://{addr}/");
        println!("JSON endpoint: http://{addr}/api/deploy-readiness");
        println!("Library: {lib_path}");

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    if let Err(err) = handle_client(stream, &library) {
                        eprintln!("request failed: {err}");
                    }
                }
                Err(err) => eprintln!("accept failed: {err}"),
            }
        }
        Ok(())
    }

    struct ZutaiLibrary {
        handle: *mut c_void,
        entry_json: EntryJsonFn,
        text_ptr: TextAbiFn,
        text_len: TextAbiFn,
    }

    impl ZutaiLibrary {
        fn open(path: &Path) -> Result<Self, String> {
            let c_path = CString::new(path.as_os_str().as_bytes())
                .map_err(|_| format!("library path contains a nul byte: {}", path.display()))?;
            let handle = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
            if handle.is_null() {
                return Err(format!("failed to open {}: {}", path.display(), dl_error()));
            }

            let entry_json = load_entry_json(handle, "zutai_entry_json")?;
            let text_ptr = load_text_abi(handle, "zutai_text_ptr")?;
            let text_len = load_text_abi(handle, "zutai_text_len")?;

            Ok(Self {
                handle,
                entry_json,
                text_ptr,
                text_len,
            })
        }

        fn entry_json(&self) -> Result<String, String> {
            let value = unsafe { (self.entry_json)() };
            self.text_to_string(value)
        }

        fn text_to_string(&self, value: i64) -> Result<String, String> {
            let ptr = unsafe { (self.text_ptr)(value) } as *const u8;
            let len = unsafe { (self.text_len)(value) };
            if len < 0 {
                return Err(format!("runtime text reported negative length {len}"));
            }
            let len = len as usize;
            if len == 0 {
                return Ok(String::new());
            }
            if ptr.is_null() {
                return Err("runtime text pointer was null".to_string());
            }
            let bytes = unsafe { slice::from_raw_parts(ptr, len) };
            std::str::from_utf8(bytes)
                .map(str::to_owned)
                .map_err(|err| format!("runtime JSON was not UTF-8: {err}"))
        }
    }

    impl Drop for ZutaiLibrary {
        fn drop(&mut self) {
            if unsafe { dlclose(self.handle) } != 0 {
                eprintln!("failed to close Zutai library: {}", dl_error());
            }
        }
    }

    fn load_entry_json(handle: *mut c_void, name: &str) -> Result<EntryJsonFn, String> {
        let symbol = load_symbol(handle, name)?;
        Ok(unsafe { std::mem::transmute::<*mut c_void, EntryJsonFn>(symbol) })
    }

    fn load_text_abi(handle: *mut c_void, name: &str) -> Result<TextAbiFn, String> {
        let symbol = load_symbol(handle, name)?;
        Ok(unsafe { std::mem::transmute::<*mut c_void, TextAbiFn>(symbol) })
    }

    fn load_symbol(handle: *mut c_void, name: &str) -> Result<*mut c_void, String> {
        let c_name = CString::new(name).expect("static symbol names do not contain nul bytes");
        unsafe {
            dlerror();
        }
        let symbol = unsafe { dlsym(handle, c_name.as_ptr()) };
        let error = unsafe { dlerror() };
        if !error.is_null() {
            return Err(format!(
                "failed to load symbol `{name}`: {}",
                dl_error_from(error)
            ));
        }
        if symbol.is_null() {
            return Err(format!("symbol `{name}` resolved to null"));
        }
        Ok(symbol)
    }

    fn dl_error() -> String {
        let error = unsafe { dlerror() };
        dl_error_from(error)
    }

    fn dl_error_from(error: *mut c_char) -> String {
        if error.is_null() {
            "unknown dynamic loader error".to_string()
        } else {
            unsafe { CStr::from_ptr(error) }
                .to_string_lossy()
                .into_owned()
        }
    }

    fn handle_client(mut stream: TcpStream, library: &ZutaiLibrary) -> io::Result<()> {
        let mut buffer = [0_u8; 4096];
        let n = stream.read(&mut buffer)?;
        let request = String::from_utf8_lossy(&buffer[..n]);
        let Some(request_line) = request.lines().next() else {
            return write_response(
                &mut stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                b"bad request\n",
                false,
            );
        };

        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let raw_path = parts.next().unwrap_or("/");
        let path = raw_path.split('?').next().unwrap_or(raw_path);
        let head_only = method == "HEAD";

        if method != "GET" && !head_only {
            return write_response(
                &mut stream,
                "405 Method Not Allowed",
                "text/plain; charset=utf-8",
                b"method not allowed\n",
                false,
            );
        }

        match path {
            "/" | "/index.html" => write_response(
                &mut stream,
                "200 OK",
                "text/html; charset=utf-8",
                INDEX_HTML.as_bytes(),
                head_only,
            ),
            "/api/deploy-readiness" => match library.entry_json() {
                Ok(json) => write_response(
                    &mut stream,
                    "200 OK",
                    "application/json; charset=utf-8",
                    json.as_bytes(),
                    head_only,
                ),
                Err(err) => {
                    let body = format!("{{\"error\":{}}}\n", json_string(&err));
                    write_response(
                        &mut stream,
                        "500 Internal Server Error",
                        "application/json; charset=utf-8",
                        body.as_bytes(),
                        head_only,
                    )
                }
            },
            _ => write_response(
                &mut stream,
                "404 Not Found",
                "text/plain; charset=utf-8",
                b"not found\n",
                head_only,
            ),
        }
    }

    fn write_response(
        stream: &mut TcpStream,
        status: &str,
        content_type: &str,
        body: &[u8],
        head_only: bool,
    ) -> io::Result<()> {
        let header = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(header.as_bytes())?;
        if !head_only {
            stream.write_all(body)?;
        }
        stream.flush()
    }

    fn json_string(value: &str) -> String {
        let mut out = String::from("\"");
        for ch in value.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
                ch => out.push(ch),
            }
        }
        out.push('"');
        out
    }
}

#[cfg(unix)]
fn main() {
    if let Err(err) = unix_host::run() {
        eprintln!("browser demo error: {err}");
        std::process::exit(1);
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("browser demo host currently requires a Unix-like dynamic loader");
    std::process::exit(1);
}
