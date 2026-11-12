//! `scope web` uses the config file the CLI resolved (`--config`, else
//! `SCOPE_CONFIG`, else the home file): its status handler names that file
//! and its save handler writes it.
//!
//! Every run uses a temp dir as the scope home, so no test touches the real
//! `~/.scope-bca`. The key in the fixtures is not a real key.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const KEY: &str = "fixture-key-not-real-0001";
const HOME_FILE: &str = "chains:\n  api_keys:\n    etherscan: \"home-file-marker-0000\"\n";

/// A running `scope web`, killed when dropped.
struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    /// The scope home. Test builds put it under `SCOPE_TEST_HOME` and
    /// ignore `SCOPE_BCA_HOME`; both are set so the paths agree.
    fn home(&self) -> PathBuf {
        self.dir.path().join("sandbox").join(".scope-bca")
    }

    fn home_file(&self) -> PathBuf {
        self.home().join("config.yaml")
    }

    fn seed_home(&self) -> Vec<u8> {
        std::fs::create_dir_all(self.home()).unwrap();
        std::fs::write(self.home_file(), HOME_FILE).unwrap();
        std::fs::read(self.home_file()).unwrap()
    }

    fn start(&self, config_flag: Option<&Path>, scope_config: Option<&Path>) -> Server {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_scope"));
        if let Some(p) = config_flag {
            cmd.arg("--config").arg(p);
        }
        cmd.args(["web", "--bind", "127.0.0.1", "--port", &port.to_string()])
            .env("SCOPE_BCA_HOME", self.home())
            .env("SCOPE_TEST_HOME", self.dir.path().join("sandbox"))
            .env("HOME", self.dir.path())
            .env("XDG_CONFIG_HOME", self.dir.path().join("config"))
            .env("XDG_DATA_HOME", self.dir.path().join("data"))
            .env_remove("SCOPE_CONFIG")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(p) = scope_config {
            cmd.env("SCOPE_CONFIG", p);
        }
        let server = Server {
            child: cmd.spawn().unwrap(),
            port,
        };
        let deadline = Instant::now() + Duration::from_secs(60);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "the server did not start");
            std::thread::sleep(Duration::from_millis(100));
        }
        server
    }
}

/// One HTTP request; returns the response body.
fn request(server: &Server, method: &str, path: &str, body: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
    raw.split_once("\r\n\r\n")
        .map_or(raw.clone(), |(_, b)| b.to_string())
}

fn save_body() -> String {
    format!("{{\"api_keys\":{{\"etherscan\":\"{KEY}\"}}}}")
}

#[test]
fn test_status_names_the_config_flag_file_and_save_writes_it() {
    // Before the fix both handlers used the home file, so a server started
    // with `--config X` showed the wrong path and saved to the wrong file.
    let env = Env::new();
    let home_before = env.seed_home();
    let x = env.dir.path().join("x.yaml");
    let server = env.start(Some(&x), None);

    let status = request(&server, "GET", "/api/config/status", "");
    assert!(status.contains("x.yaml"), "{status}");
    assert!(status.contains("\"config_exists\":false"), "{status}");

    let saved = request(&server, "POST", "/api/config", &save_body());
    assert!(saved.contains("x.yaml"), "{saved}");
    assert!(std::fs::read_to_string(&x).unwrap().contains(KEY));
    assert_eq!(std::fs::read(env.home_file()).unwrap(), home_before);
    // The key is never in a response.
    assert!(!status.contains(KEY) && !saved.contains(KEY));
}

#[test]
fn test_scope_config_variable_file_is_used_and_the_flag_beats_it() {
    // One rule for every command: flag, then variable, then home.
    let env = Env::new();
    let home_before = env.seed_home();
    let var = env.dir.path().join("var.yaml");
    let flag = env.dir.path().join("flag.yaml");

    let server = env.start(None, Some(&var));
    request(&server, "POST", "/api/config", &save_body());
    assert!(std::fs::read_to_string(&var).unwrap().contains(KEY));
    drop(server);

    let server = env.start(Some(&flag), Some(&var));
    let status = request(&server, "GET", "/api/config/status", "");
    assert!(status.contains("flag.yaml"), "{status}");
    assert_eq!(std::fs::read(env.home_file()).unwrap(), home_before);
}

#[test]
fn test_without_flag_or_variable_the_home_file_is_used() {
    let env = Env::new();
    let server = env.start(None, None);
    let status = request(&server, "GET", "/api/config/status", "");
    assert!(status.contains("config.yaml"), "{status}");
    request(&server, "POST", "/api/config", &save_body());
    assert!(
        std::fs::read_to_string(env.home_file())
            .unwrap()
            .contains(KEY)
    );
}
