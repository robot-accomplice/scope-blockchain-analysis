//! # Ghola install
//!
//! Finds the [Ghola](https://github.com/robot-accomplice/ghola) sidecar
//! binary and installs it from the project's GitHub releases.
//!
//! Nothing here asks the user: the CLI asks first and calls
//! [`install_latest`] only on a yes.
//!
//! An install takes the latest release of `robot-accomplice/ghola`, picks
//! the archive for this OS and CPU (`ghola_<ver>_<os>_<arch>.tar.gz`, or
//! `.zip` on Windows), checks its SHA-256 against the release's
//! `checksums.txt`, and only then extracts `ghola` (`ghola.exe`) from the
//! archive root to [`install_dir`]. The file is written to a temporary
//! name and renamed into place. PATH is never changed: scope runs the
//! binary by its full path.
//!
//! `checksums.txt` is not signed. The check proves the archive is the one
//! the release lists; HTTPS to GitHub is what ties the release to its
//! publisher. Every request, redirects included, must be HTTPS to a GitHub
//! host.
//!
//! The download goes through a [`ReleaseSource`], so tests use a fake
//! release and never touch the network.

use crate::error::ScopeError;
use async_trait::async_trait;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// The GitHub API URL of the latest release.
pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/robot-accomplice/ghola/releases/latest";

/// The releases page, for installing by hand.
pub const RELEASES_PAGE: &str = "https://github.com/robot-accomplice/ghola/releases";

/// The Go package of the binary. The module root has no main package.
pub const GO_PACKAGE: &str = "github.com/robot-accomplice/ghola/cmd/ghola@latest";

/// The command that builds Ghola from source.
pub const GO_INSTALL: &str = "go install github.com/robot-accomplice/ghola/cmd/ghola@latest";

/// The hosts a download may use: the API, the release page and GitHub's
/// asset stores, where release downloads redirect.
const ALLOWED_HOSTS: &[&str] = &[
    "api.github.com",
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// The largest download accepted (the archives are about 5 MB).
const MAX_DOWNLOAD_BYTES: usize = 100 * 1024 * 1024;

/// The largest binary extracted (ghola is about 13 MB).
const MAX_BINARY_BYTES: u64 = 200 * 1024 * 1024;

/// How long one download may take.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);

/// The release file that lists each asset's SHA-256.
const CHECKSUMS: &str = "checksums.txt";

/// Where a download comes from.
#[async_trait]
pub trait ReleaseSource: Send + Sync {
    /// The body of an HTTPS GET of `url`.
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, ScopeError>;
}

/// Downloads from GitHub over HTTPS. Any other URL, and any redirect to
/// one, is refused.
pub struct GithubReleases {
    client: reqwest::Client,
}

impl GithubReleases {
    /// A source for GitHub releases.
    ///
    /// # Errors
    ///
    /// Returns `Network` when the client cannot be built.
    pub fn new() -> Result<Self, ScopeError> {
        let redirects = reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                attempt.error("too many redirects")
            } else if allowed_url(attempt.url().as_str()) {
                attempt.follow()
            } else {
                let to = attempt.url().to_string();
                attempt.error(format!("redirect to {to} is not a GitHub HTTPS URL"))
            }
        });
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(redirects)
            .timeout(DOWNLOAD_TIMEOUT)
            .user_agent(concat!("scope-bca/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| {
                ScopeError::Network(format!("failed to build the download client: {e}"))
            })?;
        Ok(Self { client })
    }
}

#[async_trait]
impl ReleaseSource for GithubReleases {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, ScopeError> {
        if !allowed_url(url) {
            return Err(ScopeError::Network(format!(
                "refusing to download {url}: not a GitHub HTTPS URL"
            )));
        }
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| ScopeError::Network(format!("download of {url} failed: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ScopeError::Network(format!(
                "download of {url} failed: HTTP {status}"
            )));
        }
        if resp
            .content_length()
            .is_some_and(|n| n > MAX_DOWNLOAD_BYTES as u64)
        {
            return Err(too_large(url));
        }
        let body = resp
            .bytes()
            .await
            .map_err(|e| ScopeError::Network(format!("download of {url} failed: {e}")))?;
        if body.len() > MAX_DOWNLOAD_BYTES {
            return Err(too_large(url));
        }
        Ok(body.to_vec())
    }
}

fn too_large(url: &str) -> ScopeError {
    ScopeError::Network(format!(
        "download of {url} is larger than {MAX_DOWNLOAD_BYTES} bytes"
    ))
}

/// True when `url` is HTTPS to a GitHub host that serves releases.
pub fn allowed_url(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https" && u.host_str().is_some_and(|h| ALLOWED_HOSTS.contains(&h))
    })
}

/// An OS and CPU, in the names Ghola's release archives use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub os: &'static str,
    pub arch: &'static str,
}

impl Target {
    /// This machine, when Ghola publishes for it.
    pub fn current() -> Option<Self> {
        Self::from_rust(std::env::consts::OS, std::env::consts::ARCH)
    }

    /// The target for Rust's OS and arch names, when Ghola publishes for it.
    pub fn from_rust(os: &str, arch: &str) -> Option<Self> {
        let os = match os {
            "linux" => "linux",
            "macos" => "darwin",
            "windows" => "windows",
            _ => return None,
        };
        let arch = match arch {
            "x86_64" => "amd64",
            "aarch64" => "arm64",
            _ => return None,
        };
        Some(Self { os, arch })
    }

    fn is_zip(self) -> bool {
        self.os == "windows"
    }

    /// The release asset for `version` (without the leading `v`).
    pub fn asset_name(self, version: &str) -> String {
        let ext = if self.is_zip() { "zip" } else { "tar.gz" };
        format!("ghola_{version}_{}_{}.{ext}", self.os, self.arch)
    }

    /// The binary's file name in the archive.
    pub fn binary_name(self) -> &'static str {
        if self.is_zip() { "ghola.exe" } else { "ghola" }
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// What [`install_latest`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The binary is at `path`.
    Installed { path: PathBuf, version: String },
    /// The latest release has no archive for the target.
    NoAsset { version: String, asset: String },
}

/// Installs the latest Ghola release for `target` into `dir`.
///
/// Nothing is extracted or written before the archive's SHA-256 matches
/// `checksums.txt`.
///
/// # Errors
///
/// Returns `Network` when a download fails, the release has no
/// `checksums.txt` or does not list the archive, or the checksum does not
/// match; `IoError` when the binary cannot be written.
pub async fn install_latest(
    source: &dyn ReleaseSource,
    target: Target,
    dir: &Path,
) -> Result<Outcome, ScopeError> {
    let body = source.fetch(LATEST_RELEASE_URL).await?;
    let release: Release = serde_json::from_slice(&body)
        .map_err(|e| ScopeError::Network(format!("unreadable ghola release: {e}")))?;
    let version = release.tag_name.trim_start_matches('v').to_string();
    let name = target.asset_name(&version);
    let Some(archive) = release.assets.iter().find(|a| a.name == name) else {
        return Ok(Outcome::NoAsset {
            version,
            asset: name,
        });
    };
    let sums = release
        .assets
        .iter()
        .find(|a| a.name == CHECKSUMS)
        .ok_or_else(|| {
            ScopeError::Network(format!(
                "ghola v{version} has no {CHECKSUMS}; not installing an unchecked file"
            ))
        })?;
    let sums = source.fetch(&sums.browser_download_url).await?;
    let want = expected_sha256(&String::from_utf8_lossy(&sums), &name).ok_or_else(|| {
        ScopeError::Network(format!(
            "{CHECKSUMS} of ghola v{version} does not list {name}"
        ))
    })?;
    let bytes = source.fetch(&archive.browser_download_url).await?;
    let got: [u8; 32] = Sha256::digest(&bytes).into();
    if got != want {
        return Err(ScopeError::Network(format!(
            "{name} does not match its SHA-256 in {CHECKSUMS} (expected {}, got {}); not installing it",
            hex::encode(want),
            hex::encode(got)
        )));
    }
    let binary = extract_binary(&bytes, target)?;
    let path = write_executable(dir, target.binary_name(), &binary)?;
    Ok(Outcome::Installed { path, version })
}

/// The SHA-256 that `checksums.txt` (`<hex>  <file>` lines) gives for `file`.
pub fn expected_sha256(sums: &str, file: &str) -> Option<[u8; 32]> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        if name != file || parts.next().is_some() {
            return None;
        }
        hex::decode(hash).ok()?.try_into().ok()
    })
}

/// The binary at the root of a verified archive.
fn extract_binary(archive: &[u8], target: Target) -> Result<Vec<u8>, ScopeError> {
    let name = target.binary_name();
    let found = if target.is_zip() {
        extract_from_zip(archive, name)?
    } else {
        extract_from_tar_gz(archive, name)?
    };
    found.ok_or_else(|| ScopeError::Network(format!("the ghola archive has no {name} at its root")))
}

fn archive_error(e: impl std::fmt::Display) -> ScopeError {
    ScopeError::Network(format!("unreadable ghola archive: {e}"))
}

fn read_limited(entry: impl Read) -> Result<Vec<u8>, ScopeError> {
    let mut buf = Vec::new();
    entry
        .take(MAX_BINARY_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(archive_error)?;
    if buf.len() as u64 > MAX_BINARY_BYTES {
        return Err(archive_error("the binary is too large"));
    }
    Ok(buf)
}

fn extract_from_tar_gz(archive: &[u8], name: &str) -> Result<Option<Vec<u8>>, ScopeError> {
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries().map_err(archive_error)? {
        let entry = entry.map_err(archive_error)?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(archive_error)?;
        if path.strip_prefix(".").unwrap_or(&path) == Path::new(name) {
            return read_limited(entry).map(Some);
        }
    }
    Ok(None)
}

fn extract_from_zip(archive: &[u8], name: &str) -> Result<Option<Vec<u8>>, ScopeError> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(archive_error)?;
    for i in 0..zip.len() {
        let file = zip.by_index(i).map_err(archive_error)?;
        if file.is_file() && file.name().is_ok_and(|n| n == name) {
            return read_limited(file).map(Some);
        }
    }
    Ok(None)
}

/// Writes `bytes` to `dir/name` through a temporary file and a rename,
/// executable on Unix.
fn write_executable(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, ScopeError> {
    let context =
        |e| ScopeError::io_context(e, format!("cannot install ghola in {}", dir.display()));
    std::fs::create_dir_all(dir).map_err(context)?;
    let tmp = dir.join(format!(".{name}.{}.part", std::process::id()));
    let dest = dir.join(name);
    let written = std::fs::write(&tmp, bytes)
        .and_then(|()| make_executable(&tmp))
        .and_then(|()| std::fs::rename(&tmp, &dest));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(context(e));
    }
    Ok(dest)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Where scope installs Ghola: `<scope home>/bin`.
pub fn install_dir() -> PathBuf {
    crate::paths::home().join("bin")
}

/// The Ghola binary: on PATH, else in [`install_dir`].
pub fn find() -> Option<PathBuf> {
    locate(std::env::var_os("PATH").as_deref(), &install_dir())
}

/// The Ghola binary: in a `path` folder, else in `dir`.
pub fn locate(path: Option<&OsStr>, dir: &Path) -> Option<PathBuf> {
    path.and_then(|p| in_path(p, "ghola")).or_else(|| {
        let file = dir.join(format!("ghola{}", std::env::consts::EXE_SUFFIX));
        file.is_file().then_some(file)
    })
}

/// The program `name` in one of the `path` folders, with the platform's
/// executable suffix (`ghola.exe` on Windows). It looks in the folders
/// itself: Windows has no `which`.
pub fn in_path(path: &OsStr, name: &str) -> Option<PathBuf> {
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(path)
        .map(|dir| dir.join(&file))
        .find(|p| p.is_file())
}

/// `go` on PATH, for building Ghola when no release archive fits.
pub fn find_go() -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| in_path(&p, "go"))
}

/// Builds Ghola with `go install` into `dir` (through `GOBIN`) and returns
/// the binary's path.
///
/// # Errors
///
/// Returns `IoError` when `go` cannot run, `Other` when it fails.
pub fn go_install(go: &Path, dir: &Path) -> Result<PathBuf, ScopeError> {
    std::fs::create_dir_all(dir)
        .map_err(|e| ScopeError::io_context(e, format!("cannot create {}", dir.display())))?;
    let status = Command::new(go)
        .args(["install", GO_PACKAGE])
        .env("GOBIN", dir)
        .status()
        .map_err(|e| ScopeError::io_context(e, format!("cannot run {}", go.display())))?;
    if !status.success() {
        return Err(ScopeError::Other(format!(
            "`{GO_INSTALL}` failed ({status})"
        )));
    }
    let path = dir.join(format!("ghola{}", std::env::consts::EXE_SUFFIX));
    if path.is_file() {
        Ok(path)
    } else {
        Err(ScopeError::Other(format!(
            "`{GO_INSTALL}` did not write {}",
            path.display()
        )))
    }
}

/// How to install Ghola by hand, one line each.
pub fn manual_steps() -> Vec<String> {
    vec![
        "Run `scope setup`: it offers to download ghola from its GitHub releases.".into(),
        format!(
            "Or download the archive for this system from {RELEASES_PAGE} and put ghola in {}",
            install_dir().display()
        ),
        format!("Or build it: {GO_INSTALL}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::Write;
    use std::sync::Mutex;

    const LINUX: Target = Target {
        os: "linux",
        arch: "amd64",
    };
    const WINDOWS: Target = Target {
        os: "windows",
        arch: "amd64",
    };
    const BINARY: &[u8] = b"\x7fELF fake ghola";

    /// A release served from memory. It records each URL fetched.
    struct FakeRelease {
        files: HashMap<String, Vec<u8>>,
        fetched: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ReleaseSource for FakeRelease {
        async fn fetch(&self, url: &str) -> Result<Vec<u8>, ScopeError> {
            self.fetched.lock().unwrap().push(url.to_string());
            self.files
                .get(url)
                .cloned()
                .ok_or_else(|| ScopeError::Network(format!("HTTP 404 for {url}")))
        }
    }

    fn url(file: &str) -> String {
        format!("https://github.com/robot-accomplice/ghola/releases/download/v0.6.2/{file}")
    }

    fn tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, name, *data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, data) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    /// A v0.6.2 release with `asset` (named for `target`) and a
    /// `checksums.txt` that gives `listed` as its hash.
    fn release(target: Target, archive: Vec<u8>, listed: Option<String>) -> FakeRelease {
        let name = target.asset_name("0.6.2");
        let hash = listed.unwrap_or_else(|| hex::encode(Sha256::digest(&archive)));
        let sums = format!("{}  other.deb\n{hash}  {name}\n", "0".repeat(64));
        let json = serde_json::json!({
            "tag_name": "v0.6.2",
            "assets": [
                {"name": CHECKSUMS, "browser_download_url": url(CHECKSUMS)},
                {"name": name, "browser_download_url": url(&name)},
            ]
        });
        let files = HashMap::from([
            (
                LATEST_RELEASE_URL.to_string(),
                json.to_string().into_bytes(),
            ),
            (url(CHECKSUMS), sums.into_bytes()),
            (url(&name), archive),
        ]);
        FakeRelease {
            files,
            fetched: Mutex::default(),
        }
    }

    #[test]
    fn test_target_names_match_the_release_assets() {
        let mac = Target::from_rust("macos", "aarch64").unwrap();
        assert_eq!(mac.asset_name("0.6.2"), "ghola_0.6.2_darwin_arm64.tar.gz");
        assert_eq!(mac.binary_name(), "ghola");
        let win = Target::from_rust("windows", "x86_64").unwrap();
        assert_eq!(win.asset_name("0.6.2"), "ghola_0.6.2_windows_amd64.zip");
        assert_eq!(win.binary_name(), "ghola.exe");
        assert_eq!(Target::from_rust("freebsd", "x86_64"), None);
        assert_eq!(Target::from_rust("linux", "riscv64"), None);
    }

    #[test]
    fn test_only_github_https_urls_are_allowed() {
        assert!(allowed_url(LATEST_RELEASE_URL));
        assert!(allowed_url(&url("checksums.txt")));
        assert!(allowed_url(
            "https://release-assets.githubusercontent.com/x?sig=1"
        ));
        assert!(!allowed_url("http://github.com/robot-accomplice/ghola"));
        assert!(!allowed_url("https://github.com.evil.example/x"));
        assert!(!allowed_url("https://example.com/ghola.tar.gz"));
        assert!(!allowed_url("not a url"));
    }

    #[test]
    fn test_expected_sha256_reads_the_named_line() {
        let a = "ab".repeat(32);
        let sums = format!("{}  ghola_x.zip\n{a}  ghola_y.tar.gz\n", "cd".repeat(32));
        assert_eq!(expected_sha256(&sums, "ghola_y.tar.gz"), Some([0xab; 32]));
        assert_eq!(
            expected_sha256(&format!("{a} *f.zip"), "f.zip"),
            Some([0xab; 32])
        );
        assert_eq!(expected_sha256(&sums, "missing.zip"), None);
        assert_eq!(expected_sha256("zz  f.zip", "f.zip"), None);
    }

    #[tokio::test]
    async fn test_install_tar_gz_writes_only_the_binary() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz(&[("LICENSE", b"MIT"), ("ghola", BINARY)]);
        let source = release(LINUX, archive, None);
        let outcome = install_latest(&source, LINUX, dir.path()).await.unwrap();
        let path = dir.path().join("ghola");
        assert_eq!(
            outcome,
            Outcome::Installed {
                path: path.clone(),
                version: "0.6.2".into()
            }
        );
        assert_eq!(std::fs::read(&path).unwrap(), BINARY);
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("ghola")]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    #[tokio::test]
    async fn test_install_zip_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let archive = zip_of(&[("README.md", b"hi"), ("ghola.exe", BINARY)]);
        let source = release(WINDOWS, archive, None);
        let outcome = install_latest(&source, WINDOWS, dir.path()).await.unwrap();
        let path = dir.path().join("ghola.exe");
        assert!(matches!(outcome, Outcome::Installed { path: p, .. } if p == path));
        assert_eq!(std::fs::read(&path).unwrap(), BINARY);
    }

    #[tokio::test]
    async fn test_a_bad_checksum_installs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz(&[("ghola", BINARY)]);
        let source = release(LINUX, archive, Some("11".repeat(32)));
        let err = install_latest(&source, LINUX, dir.path())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("SHA-256"), "{err}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn test_an_unlisted_archive_installs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = release(LINUX, tar_gz(&[("ghola", BINARY)]), None);
        source
            .files
            .insert(url(CHECKSUMS), b"00  something-else.tar.gz\n".to_vec());
        let err = install_latest(&source, LINUX, dir.path())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("does not list"), "{err}");
        // The archive is never downloaded.
        let fetched = source.fetched.lock().unwrap();
        assert!(
            !fetched.iter().any(|u| u.ends_with(".tar.gz")),
            "{fetched:?}"
        );
    }

    #[tokio::test]
    async fn test_a_release_without_checksums_installs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut source = release(LINUX, tar_gz(&[("ghola", BINARY)]), None);
        let json = serde_json::json!({
            "tag_name": "v0.6.2",
            "assets": [{"name": LINUX.asset_name("0.6.2"),
                        "browser_download_url": url(&LINUX.asset_name("0.6.2"))}]
        });
        source
            .files
            .insert(LATEST_RELEASE_URL.into(), json.to_string().into_bytes());
        let err = install_latest(&source, LINUX, dir.path())
            .await
            .unwrap_err();
        assert!(err.to_string().contains(CHECKSUMS), "{err}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn test_no_asset_for_the_target_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let source = release(LINUX, tar_gz(&[("ghola", BINARY)]), None);
        let mac = Target::from_rust("macos", "x86_64").unwrap();
        let outcome = install_latest(&source, mac, dir.path()).await.unwrap();
        assert_eq!(
            outcome,
            Outcome::NoAsset {
                version: "0.6.2".into(),
                asset: "ghola_0.6.2_darwin_amd64.tar.gz".into()
            }
        );
    }

    #[tokio::test]
    async fn test_a_binary_below_the_root_is_not_taken() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz(&[("bin/ghola", BINARY)]);
        let source = release(LINUX, archive, None);
        let err = install_latest(&source, LINUX, dir.path())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no ghola"), "{err}");
    }

    #[test]
    fn test_locate_prefers_path_then_the_install_dir() {
        let file = format!("ghola{}", std::env::consts::EXE_SUFFIX);
        let on_path = tempfile::tempdir().unwrap();
        let installed = tempfile::tempdir().unwrap();
        let empty = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([empty.path(), on_path.path()]).unwrap();
        assert_eq!(locate(Some(&path), installed.path()), None);
        std::fs::write(installed.path().join(&file), b"x").unwrap();
        assert_eq!(
            locate(Some(&path), installed.path()),
            Some(installed.path().join(&file))
        );
        std::fs::write(on_path.path().join(&file), b"x").unwrap();
        assert_eq!(
            locate(Some(&path), installed.path()),
            Some(on_path.path().join(&file))
        );
        assert_eq!(
            locate(None, installed.path()),
            Some(installed.path().join(&file))
        );
    }

    #[test]
    fn test_in_path_finds_a_program_in_any_path_folder() {
        let empty = tempfile::tempdir().unwrap();
        let with = tempfile::tempdir().unwrap();
        let program = format!("ghola{}", std::env::consts::EXE_SUFFIX);
        std::fs::write(with.path().join(&program), b"").unwrap();
        let path = std::env::join_paths([empty.path(), with.path()]).unwrap();
        assert_eq!(in_path(&path, "ghola"), Some(with.path().join(&program)));
        let only_empty = std::env::join_paths([empty.path()]).unwrap();
        assert_eq!(in_path(&only_empty, "ghola"), None);
    }

    #[test]
    fn test_manual_steps_use_the_cmd_package() {
        let steps = manual_steps().join("\n");
        assert!(steps.contains("/cmd/ghola@latest"), "{steps}");
        assert!(steps.contains(RELEASES_PAGE), "{steps}");
    }
}
