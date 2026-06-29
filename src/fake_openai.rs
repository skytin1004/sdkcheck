use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde_json::json;

use crate::runner::{Backend, DEFAULT_DOCKER_IMAGE};

const DOCKER_FAKE_OPENAI_SCRIPT: &str = r##"from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json


def fake_translation_for(request_body: str) -> str:
    if "# Setup" in request_body:
        return "# Setup\n\nSet the `EXAMPLE_API_KEY` environment variable.\n"
    return "# Sample Project\n\nThis is a sample document for sdkcheck Co-op Translator dogfood.\n\n```python\nprint(\"hello from sdkcheck\")\n```\n\nSee [Setup](docs/setup.md) for details.\n"


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("content-length", "0") or "0")
        body = self.rfile.read(length).decode("utf-8", errors="replace")
        content = fake_translation_for(body)
        payload = {
            "id": "chatcmpl-sdkcheck-fake",
            "object": "chat.completion",
            "created": 0,
            "model": "sdkcheck-fake-model",
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": content},
                    "finish_reason": "stop",
                }
            ],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2},
        }
        encoded = json.dumps(payload).encode("utf-8")
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(encoded)))
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(encoded)

    def do_GET(self):
        encoded = b'{"ok": true}'
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, fmt, *args):
        return


ThreadingHTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
"##;

pub enum FakeOpenAiServer {
    Host(HostFakeOpenAiServer),
    Docker(DockerFakeOpenAiServer),
}

impl FakeOpenAiServer {
    pub fn start(backend: Backend, run_dir: &Path) -> Result<Self> {
        match backend {
            Backend::Local => Ok(Self::Host(HostFakeOpenAiServer::start()?)),
            Backend::Docker => Ok(Self::Docker(DockerFakeOpenAiServer::start(run_dir)?)),
        }
    }

    pub fn base_url(&self) -> String {
        match self {
            Self::Host(server) => server.base_url(),
            Self::Docker(server) => server.base_url(),
        }
    }

    pub fn docker_network(&self) -> Option<String> {
        match self {
            Self::Host(_) => None,
            Self::Docker(server) => Some(server.network.clone()),
        }
    }
}

pub struct HostFakeOpenAiServer {
    port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl HostFakeOpenAiServer {
    pub fn start() -> Result<Self> {
        let bind_addr = "127.0.0.1:0";
        let listener = TcpListener::bind(bind_addr)
            .with_context(|| format!("failed to bind fake OpenAI server on {bind_addr}"))?;
        let port = listener
            .local_addr()
            .context("failed to read fake OpenAI server address")?
            .port();
        listener
            .set_nonblocking(true)
            .context("failed to set fake OpenAI server nonblocking mode")?;

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);

        let handle = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        thread::spawn(move || {
                            let _ = handle_connection(stream);
                        });
                    }
                    Err(_) => {
                        thread::sleep(Duration::from_millis(25));
                    }
                }
            }
        });

        Ok(Self {
            port,
            stop,
            handle: Some(handle),
        })
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }
}

impl Drop for HostFakeOpenAiServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(SocketAddr::from(([127, 0, 0, 1], self.port)));

        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub struct DockerFakeOpenAiServer {
    network: String,
    container: String,
}

impl DockerFakeOpenAiServer {
    pub fn start(run_dir: &Path) -> Result<Self> {
        let suffix = docker_safe_name(
            run_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("run"),
        );
        let network = format!("sdkcheck-{suffix}");
        let container = format!("sdkcheck-fake-openai-{suffix}");
        let script_path = run_dir.join(".sdkcheck-fake-openai.py");

        fs::write(&script_path, DOCKER_FAKE_OPENAI_SCRIPT).with_context(|| {
            format!(
                "failed to write Docker fake OpenAI script `{}`",
                script_path.display()
            )
        })?;

        cleanup_docker_resource("rm", &["-f", &container]);
        cleanup_docker_resource("network", &["rm", &network]);

        let network_output = Command::new("docker")
            .args(["network", "create", &network])
            .output()
            .context("failed to start Docker network creation")?;
        ensure_success("create Docker fake OpenAI network", network_output)?;

        let mount = format!("type=bind,source={},target=/work", run_dir.display());
        let run_output = Command::new("docker")
            .arg("run")
            .arg("-d")
            .arg("--rm")
            .arg("--name")
            .arg(&container)
            .arg("--network")
            .arg(&network)
            .arg("--security-opt")
            .arg("no-new-privileges")
            .arg("--pids-limit")
            .arg("128")
            .arg("--memory")
            .arg("256m")
            .arg("--cpus")
            .arg("1")
            .arg("--mount")
            .arg(mount)
            .arg("-w")
            .arg("/work")
            .arg(DEFAULT_DOCKER_IMAGE)
            .arg("python")
            .arg("-u")
            .arg(".sdkcheck-fake-openai.py")
            .output()
            .context("failed to start Docker fake OpenAI container")?;
        ensure_success("start Docker fake OpenAI container", run_output)?;

        let server = Self { network, container };
        server.wait_until_ready()?;
        Ok(server)
    }

    pub fn base_url(&self) -> String {
        format!("http://{}:8080/v1", self.container)
    }

    fn wait_until_ready(&self) -> Result<()> {
        let url = format!("http://{}:8080/v1/chat/completions", self.container);
        let probe = format!(
            r#"import json, urllib.request
payload = {{"model": "sdkcheck-fake-model", "messages": [{{"role": "user", "content": "ping"}}]}}
request = urllib.request.Request({url:?}, data=json.dumps(payload).encode("utf-8"), headers={{"content-type": "application/json"}}, method="POST")
print(urllib.request.urlopen(request, timeout=5).status)
"#
        );

        let mut last_error = String::new();
        for _ in 0..30 {
            let output = Command::new("docker")
                .arg("run")
                .arg("--rm")
                .arg("--network")
                .arg(&self.network)
                .arg("--security-opt")
                .arg("no-new-privileges")
                .arg("--pids-limit")
                .arg("64")
                .arg("--memory")
                .arg("128m")
                .arg("--cpus")
                .arg("1")
                .arg(DEFAULT_DOCKER_IMAGE)
                .arg("python")
                .arg("-c")
                .arg(&probe)
                .output()
                .context("failed to start Docker fake OpenAI readiness probe")?;

            if output.status.success() {
                return Ok(());
            }

            last_error = format!(
                "stdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout).trim_end(),
                String::from_utf8_lossy(&output.stderr).trim_end()
            );
            thread::sleep(Duration::from_millis(500));
        }

        Err(anyhow!(
            "Docker fake OpenAI container `{}` did not become ready\n{}",
            self.container,
            last_error
        ))
    }
}

impl Drop for DockerFakeOpenAiServer {
    fn drop(&mut self) {
        cleanup_docker_resource("rm", &["-f", &self.container]);
        cleanup_docker_resource("network", &["rm", &self.network]);
    }
}

fn docker_safe_name(input: &str) -> String {
    input
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn cleanup_docker_resource(command: &str, args: &[&str]) {
    let _ = Command::new("docker").arg(command).args(args).output();
}

fn ensure_success(action: &str, output: std::process::Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    Err(anyhow!(
        "failed to {action}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout).trim_end(),
        String::from_utf8_lossy(&output.stderr).trim_end()
    ))
}

fn handle_connection(mut stream: TcpStream) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .context("failed to set fake OpenAI read timeout")?;

    let request = read_http_request(&mut stream)?;
    let body = request_body(&request);
    let content = fake_translation_for(body);
    let response_body = json!({
        "id": "chatcmpl-sdkcheck-fake",
        "object": "chat.completion",
        "created": 0,
        "model": "sdkcheck-fake-model",
        "choices": [
            {
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content
                },
                "finish_reason": "stop"
            }
        ],
        "usage": {
            "prompt_tokens": 1,
            "completion_tokens": 1,
            "total_tokens": 2
        }
    })
    .to_string();

    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );

    stream
        .write_all(response.as_bytes())
        .context("failed to write fake OpenAI response")
}

fn read_http_request(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];

    loop {
        let read = stream
            .read(&mut buffer)
            .context("failed to read fake OpenAI request")?;

        if read == 0 {
            break;
        }

        request.extend_from_slice(&buffer[..read]);

        if let Some(header_end) = header_end(&request) {
            let content_length = content_length(&request[..header_end]).unwrap_or(0);
            let body_len = request.len().saturating_sub(header_end + 4);

            if body_len >= content_length {
                break;
            }
        }
    }

    Ok(request)
}

fn request_body(request: &[u8]) -> &str {
    let Some(header_end) = header_end(request) else {
        return "";
    };

    std::str::from_utf8(&request[header_end + 4..]).unwrap_or("")
}

fn header_end(request: &[u8]) -> Option<usize> {
    request.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(headers: &[u8]) -> Option<usize> {
    let headers = std::str::from_utf8(headers).ok()?;

    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;

        if name.eq_ignore_ascii_case("content-length") {
            value.trim().parse().ok()
        } else {
            None
        }
    })
}

pub fn fake_translation_for(request_body: &str) -> &'static str {
    if request_body.contains("# Setup") {
        "# Setup\n\nSet the `EXAMPLE_API_KEY` environment variable.\n"
    } else {
        "# Sample Project\n\nThis is a sample document for sdkcheck Co-op Translator dogfood.\n\n```python\nprint(\"hello from sdkcheck\")\n```\n\nSee [Setup](docs/setup.md) for details.\n"
    }
}

#[cfg(test)]
mod tests {
    use super::fake_translation_for;

    #[test]
    fn returns_setup_specific_content() {
        let content = fake_translation_for("please translate\n# Setup\n");

        assert!(content.contains("EXAMPLE_API_KEY"));
        assert!(!content.contains("```python"));
    }
}
