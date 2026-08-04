use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const EXAMPLE_MOD_NAME: &str = "@fleet-example";
const EXAMPLE_FILE_REL: &str = "addons\\hello.txt";
const EXAMPLE_FILE_ROUTE: &str = "/@fleet-example/addons/hello.txt";
const EXAMPLE_FILE_BYTES: &[u8] = b"fleet dummy file\n";
const EXAMPLE_FILE_MD5: &str = "7dc1773e58b61108bc3f40abdb29eaa4";
const EXAMPLE_MOD_MD5: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

pub struct ExampleSwiftyRepoServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl ExampleSwiftyRepoServer {
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let repo = Arc::new(ExampleSwiftyRepo::new());
        let handle = {
            let stop = Arc::clone(&stop);
            thread::spawn(move || serve(listener, repo, stop))
        };

        Ok(Self {
            addr,
            stop,
            handle: Some(handle),
        })
    }

    pub fn repo_url(&self) -> String {
        format!("http://{}/repo.json", self.addr)
    }

    pub fn example_file_target_path(&self) -> PathBuf {
        PathBuf::from(EXAMPLE_MOD_NAME)
            .join("addons")
            .join("hello.txt")
    }

    pub fn example_file_bytes(&self) -> &'static [u8] {
        EXAMPLE_FILE_BYTES
    }
}

impl Drop for ExampleSwiftyRepoServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.addr);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct ExampleSwiftyRepo {
    repo_json: Vec<u8>,
    mod_srf: Vec<u8>,
}

impl ExampleSwiftyRepo {
    fn new() -> Self {
        let mod_checksum = md5_digest(EXAMPLE_MOD_MD5);
        let part = swifty_artifacts::SrfPart {
            path: format!(
                "{}_{}",
                EXAMPLE_FILE_REL.replace('\\', "/"),
                EXAMPLE_FILE_BYTES.len()
            ),
            start: 0,
            length: EXAMPLE_FILE_BYTES.len() as u64,
            checksum: md5_digest(EXAMPLE_FILE_MD5),
        };
        let file_checksum = swifty_artifacts::file_md5_from_parts(std::slice::from_ref(&part));
        let repo = swifty_artifacts::RepoSpec {
            repo_name: "fleet-example-test-repo".to_string(),
            checksum: "0000000000000000000000000000000000000000".to_string(),
            required_mods: vec![swifty_artifacts::RepoMod {
                mod_name: EXAMPLE_MOD_NAME.to_string(),
                checksum: mod_checksum,
                enabled: true,
            }],
            optional_mods: vec![],
            icon_image_path: None,
            icon_image_checksum: None,
            repo_image_path: None,
            repo_image_checksum: None,
            required_dlcs: vec![],
            client_parameters: String::new(),
            repo_basic_authentication: None,
            version: String::new(),
            servers: vec![],
        };
        let mod_manifest = swifty_artifacts::SrfMod {
            name: EXAMPLE_MOD_NAME.to_string(),
            checksum: mod_checksum,
            files: vec![swifty_artifacts::SrfFile {
                path: EXAMPLE_FILE_REL.to_string(),
                length: EXAMPLE_FILE_BYTES.len() as u64,
                checksum: file_checksum,
                r#type: None,
                parts: vec![part],
            }],
        };

        Self {
            repo_json: serde_json::to_vec(&repo).expect("serialize repo.json"),
            mod_srf: serde_json::to_vec(&mod_manifest).expect("serialize mod.srf"),
        }
    }

    fn response(&self, request: &str) -> Vec<u8> {
        let request = HttpRequest::parse(request);
        let (status, content_type, body) = match request.path.as_str() {
            "/repo.json" => (
                "200 OK",
                "application/json",
                self.repo_json.as_slice().to_vec(),
            ),
            "/@fleet-example/mod.srf" => (
                "200 OK",
                "application/json",
                self.mod_srf.as_slice().to_vec(),
            ),
            EXAMPLE_FILE_ROUTE => (
                "200 OK",
                "application/octet-stream",
                EXAMPLE_FILE_BYTES.to_vec(),
            ),
            _ => ("404 Not Found", "text/plain", b"not found".to_vec()),
        };

        let (status, body, content_range) =
            if request.path == EXAMPLE_FILE_ROUTE && status.starts_with("200") {
                match requested_range(request.raw, body.len()) {
                    Some((start, end)) => (
                        "206 Partial Content",
                        body[start..=end].to_vec(),
                        Some(format!(
                            "Content-Range: bytes {start}-{end}/{}\r\n",
                            body.len()
                        )),
                    ),
                    None => (status, body, None),
                }
            } else {
                (status, body, None)
            };

        http_response(
            status,
            content_type,
            body,
            content_range,
            request.method == "HEAD",
        )
    }
}

struct HttpRequest<'a> {
    method: &'a str,
    path: String,
    raw: &'a str,
}

impl<'a> HttpRequest<'a> {
    fn parse(raw: &'a str) -> Self {
        let (method, raw_path) = raw
            .lines()
            .next()
            .and_then(|line| {
                let mut parts = line.split_whitespace();
                Some((parts.next()?, parts.next()?))
            })
            .unwrap_or(("GET", "/"));
        let path = raw_path.split_once('?').map_or(raw_path, |(path, _)| path);

        Self {
            method,
            path: path.to_string(),
            raw,
        }
    }
}

fn serve(listener: TcpListener, repo: Arc<ExampleSwiftyRepo>, stop: Arc<AtomicBool>) {
    for stream in listener.incoming() {
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let Ok(mut stream) = stream else {
            break;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut request = [0u8; 8192];
        let bytes_read = stream.read(&mut request).unwrap_or(0);
        if bytes_read == 0 {
            continue;
        }

        let request = String::from_utf8_lossy(&request[..bytes_read]);
        let response = repo.response(&request);
        let _ = stream.write_all(&response);
        let _ = stream.flush();
    }
}

fn http_response(
    status: &str,
    content_type: &str,
    body: Vec<u8>,
    content_range: Option<String>,
    head_only: bool,
) -> Vec<u8> {
    let content_len = body.len();
    let response_body = if head_only { Vec::new() } else { body };
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
        content_len,
        content_range.unwrap_or_default()
    )
    .into_bytes();
    response.extend_from_slice(&response_body);
    response
}

fn requested_range(request: &str, len: usize) -> Option<(usize, usize)> {
    for line in request.lines() {
        let lower = line.to_ascii_lowercase();
        let Some(spec) = lower.strip_prefix("range: bytes=") else {
            continue;
        };
        let (start, end) = spec.split_once('-')?;
        let start = start.parse::<usize>().ok()?;
        let end = if end.is_empty() {
            len.checked_sub(1)?
        } else {
            end.parse::<usize>().ok()?
        };
        if start <= end && end < len {
            return Some((start, end));
        }
    }
    None
}

fn md5_digest(hex: &str) -> swifty_artifacts::Md5Digest {
    swifty_artifacts::Md5Digest::parse_hex(hex).expect("valid md5")
}
