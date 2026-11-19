#![expect(
    clippy::disallowed_types,
    reason = "sync CLI `upgrade` command: blocking std::io::Write and std::io::BufReader for \
              release download + prompt output"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "sync CLI `upgrade` command: interactive confirm prompt reads from std::io::stdin"
)]

use std::fs;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use fabro_types::settings::CliNamespace;
use fabro_types::settings::cli::OutputFormat;
use fabro_util::printer::Printer;
use semver::Version;
use sha2::{Digest, Sha256};
use tokio::process::Command as TokioCommand;
use tokio::task::JoinHandle;
use tracing::debug;

use crate::args::UpgradeArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

// ── Download backend abstraction ───────────────────────────────────────────

/// Upstream repository. `release_repo()` falls back here when the running
/// version carries no `-fork` pre-release suffix.
const GITHUB_REPO: &str = "fabro-sh/fabro";

/// Repository hosting the fork's `-fork.N` releases (work#1465).
const FORK_GITHUB_REPO: &str = "denkhaus/fabro";

/// Resolve the GitHub repository the upgrade door targets (work#1465).
///
/// Fork builds carry a `-fork.N` suffix in their version
/// (`0.362.0-fork.42-a08b4c3111e7`) and release into `denkhaus/fabro`,
/// while upstream builds release into [`GITHUB_REPO`]. The running version
/// string decides, so the door reaches the fork's releases for fork builds
/// and keeps working against upstream for upstream consumers.
fn release_repo() -> &'static str {
    if env!("CARGO_PKG_VERSION").contains("-fork") {
        FORK_GITHUB_REPO
    } else {
        GITHUB_REPO
    }
}

enum Backend {
    Gh,
    Http(fabro_http::HttpClient),
}

fn http_client() -> Result<fabro_http::HttpClient> {
    fabro_http::HttpClientBuilder::new()
        .user_agent("fabro-cli")
        .build()
        .context("failed to build HTTP client")
}

// ── Install source detection ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallSource {
    Tarball,
    Brew { channel: BrewChannel },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrewChannel {
    Stable,
    Nightly,
}

impl InstallSource {
    fn cache_tag(self) -> &'static str {
        match self {
            Self::Tarball => "tarball",
            Self::Brew {
                channel: BrewChannel::Stable,
            } => "brew-stable",
            Self::Brew {
                channel: BrewChannel::Nightly,
            } => "brew-nightly",
        }
    }
}

fn upgrade_command_for(source: InstallSource) -> &'static str {
    match source {
        InstallSource::Tarball => "fabro upgrade",
        InstallSource::Brew {
            channel: BrewChannel::Stable,
        } => "brew upgrade fabro",
        InstallSource::Brew {
            channel: BrewChannel::Nightly,
        } => "brew upgrade fabro-nightly",
    }
}

fn detect_install_source(current_exe: &Path) -> InstallSource {
    let s = current_exe.to_string_lossy();
    if s.contains("/Cellar/fabro-nightly/") {
        InstallSource::Brew {
            channel: BrewChannel::Nightly,
        }
    } else if s.contains("/Cellar/fabro/") {
        InstallSource::Brew {
            channel: BrewChannel::Stable,
        }
    } else {
        InstallSource::Tarball
    }
}

// ── Homebrew tap manifest ──────────────────────────────────────────────────

const TAP_VERSIONS_URL: &str =
    "https://raw.githubusercontent.com/fabro-sh/homebrew-tap/main/versions.json";

fn parse_tap_versions(body: &str, channel: BrewChannel) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(body).context("parsing tap versions.json")?;
    let key = match channel {
        BrewChannel::Stable => "stable",
        BrewChannel::Nightly => "nightly",
    };
    v.get(key)
        .and_then(|c| c.get("version"))
        .and_then(|s| s.as_str())
        .map(str::to_string)
        .with_context(|| format!("missing {key}.version in tap versions.json"))
}

async fn fetch_tap_version(
    client: &fabro_http::HttpClient,
    channel: BrewChannel,
) -> Result<String> {
    let resp = client
        .get(TAP_VERSIONS_URL)
        .send()
        .await
        .context("failed to fetch tap versions.json")?;
    if !resp.status().is_success() {
        bail!("tap versions.json returned HTTP {}", resp.status());
    }
    let body = resp.text().await?;
    parse_tap_versions(&body, channel)
}

impl Backend {
    /// Fetch the latest published release together with its attached asset
    /// names (work#1465: the assets ride along so the exact version-prefixed
    /// asset names of fork releases can be resolved before downloading).
    async fn fetch_latest_release(&self) -> Result<ReleaseSummary> {
        let repo = release_repo();
        match self {
            Self::Gh => {
                let output = TokioCommand::new("gh")
                    .args(["api", &format!("repos/{repo}/releases/latest")])
                    .output()
                    .await
                    .context("failed to run `gh api repos/.../releases/latest`")?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    bail!("gh api repos/.../releases/latest failed: {stderr}");
                }
                let release: ReleaseSummary = serde_json::from_slice(&output.stdout)
                    .context("failed to parse gh latest release JSON")?;
                Ok(release)
            }
            Self::Http(client) => {
                let url = format!("https://api.github.com/repos/{repo}/releases/latest");
                let resp = client
                    .get(&url)
                    .send()
                    .await
                    .context("failed to fetch latest release from GitHub API")?;
                if !resp.status().is_success() {
                    bail!(
                        "GitHub API returned status {} when fetching latest release",
                        resp.status()
                    );
                }
                let release: ReleaseSummary = resp.json().await?;
                Ok(release)
            }
        }
    }

    /// Fetch a single release by tag, with its attached asset names
    /// (work#1465: the `--version` path resolves assets from the release
    /// instead of deriving version-less asset names).
    async fn fetch_release_by_tag(&self, tag: &str) -> Result<ReleaseSummary> {
        let repo = release_repo();
        match self {
            Self::Gh => {
                let output = TokioCommand::new("gh")
                    .args(["api", &format!("repos/{repo}/releases/tags/{tag}")])
                    .output()
                    .await
                    .context("failed to run `gh api repos/.../releases/tags/...`")?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    bail!("gh api repos/.../releases/tags/... failed: {stderr}");
                }
                let release: ReleaseSummary = serde_json::from_slice(&output.stdout)
                    .context("failed to parse gh release JSON")?;
                Ok(release)
            }
            Self::Http(client) => {
                let url = format!("https://api.github.com/repos/{repo}/releases/tags/{tag}");
                let resp =
                    client.get(&url).send().await.with_context(|| {
                        format!("failed to fetch release {tag} from GitHub API")
                    })?;
                if !resp.status().is_success() {
                    bail!(
                        "GitHub API returned status {} when fetching release {tag} \
                         (pass the exact release tag)",
                        resp.status()
                    );
                }
                let release: ReleaseSummary = resp.json().await?;
                Ok(release)
            }
        }
    }

    async fn fetch_releases(&self) -> Result<Vec<ReleaseSummary>> {
        let repo = release_repo();
        match self {
            Self::Gh => {
                let output = TokioCommand::new("gh")
                    .args(["api", &format!("repos/{repo}/releases")])
                    .output()
                    .await
                    .context("failed to run `gh api repos/.../releases`")?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    bail!("gh api repos/.../releases failed: {stderr}");
                }
                let releases: Vec<ReleaseSummary> = serde_json::from_slice(&output.stdout)
                    .context("failed to parse gh releases JSON")?;
                Ok(releases)
            }
            Self::Http(client) => {
                let url = format!("https://api.github.com/repos/{repo}/releases");
                let resp = client
                    .get(&url)
                    .send()
                    .await
                    .context("failed to fetch releases from GitHub API")?;
                if !resp.status().is_success() {
                    bail!(
                        "GitHub API returned status {} when fetching releases",
                        resp.status()
                    );
                }
                let releases: Vec<ReleaseSummary> = resp.json().await?;
                Ok(releases)
            }
        }
    }

    async fn download_release(&self, tag: &str, asset: &str, dest_dir: &Path) -> Result<PathBuf> {
        let dest = dest_dir.join(asset);
        let repo = release_repo();
        match self {
            Self::Gh => {
                let status = TokioCommand::new("gh")
                    .args([
                        "release",
                        "download",
                        tag,
                        "--repo",
                        repo,
                        "--pattern",
                        asset,
                        "--dir",
                        &dest_dir.to_string_lossy(),
                        "--clobber",
                    ])
                    .status()
                    .await
                    .context("failed to run `gh release download`")?;
                if !status.success() {
                    bail!("gh release download failed with exit code {status}");
                }
            }
            Self::Http(client) => {
                let url = format!("https://github.com/{repo}/releases/download/{tag}/{asset}");
                let resp = client
                    .get(&url)
                    .send()
                    .await
                    .with_context(|| format!("failed to download {url}"))?;
                if !resp.status().is_success() {
                    bail!("download failed: HTTP {}", resp.status());
                }
                let bytes = resp.bytes().await?;
                let mut file = fs::File::create(&dest)
                    .with_context(|| format!("creating download file {}", dest.display()))?;
                file.write_all(&bytes)
                    .with_context(|| format!("writing download to {}", dest.display()))?;
            }
        }
        Ok(dest)
    }
}

async fn select_backend() -> Result<Backend> {
    select_backend_for_gh_command("gh").await
}

async fn select_backend_for_gh_command(gh_command: &str) -> Result<Backend> {
    // Check if gh is available
    let gh_version = TokioCommand::new(gh_command)
        .arg("--version")
        .output()
        .await;
    let Ok(output) = gh_version else {
        debug!("gh CLI not found, using HTTP backend");
        return Ok(Backend::Http(http_client()?));
    };
    if !output.status.success() {
        debug!("gh --version failed, using HTTP backend");
        return Ok(Backend::Http(http_client()?));
    }

    // Check if gh is authenticated
    let auth_status = TokioCommand::new(gh_command)
        .args(["auth", "status"])
        .output()
        .await;
    match auth_status {
        Ok(o) if o.status.success() => {
            debug!("gh CLI available and authenticated, using Gh backend");
            Ok(Backend::Gh)
        }
        _ => {
            debug!("gh not authenticated, using HTTP backend");
            Ok(Backend::Http(http_client()?))
        }
    }
}

// ── Platform detection ─────────────────────────────────────────────────────

fn detect_target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("linux", "x86_64") => Ok(match detect_linux_libc() {
            Libc::Musl => "x86_64-unknown-linux-musl",
            Libc::Gnu => "x86_64-unknown-linux-gnu",
        }),
        ("linux", "aarch64") => Ok(match detect_linux_libc() {
            Libc::Musl => "aarch64-unknown-linux-musl",
            Libc::Gnu => "aarch64-unknown-linux-gnu",
        }),
        (os, arch) => bail!("unsupported platform: {os}/{arch}"),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Libc {
    Gnu,
    Musl,
}

/// Parse `ldd --version` output to determine the host libc flavor.
///
/// glibc's ldd writes to stdout ("ldd (Ubuntu GLIBC 2.35...)");
/// musl's ldd writes to stderr ("musl libc (x86_64)\nVersion 1.2.4").
/// Callers concatenate both streams before passing in.
fn parse_ldd_libc(output: &str) -> Libc {
    if output.to_ascii_lowercase().contains("musl") {
        Libc::Musl
    } else {
        Libc::Gnu
    }
}

fn detect_linux_libc() -> Libc {
    #[expect(
        clippy::disallowed_methods,
        reason = "one-shot libc detection at upgrade startup; async overhead is unwarranted"
    )]
    let result = std::process::Command::new("ldd").arg("--version").output();
    let Ok(output) = result else {
        return Libc::Gnu;
    };
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    parse_ldd_libc(&combined)
}

// ── Version helpers ────────────────────────────────────────────────────────

fn parse_version_from_tag(tag: &str) -> Result<Version> {
    let stripped = tag.strip_prefix('v').unwrap_or(tag);
    Version::parse(stripped).with_context(|| format!("invalid version: {tag}"))
}

/// True when a version is a fork build: its pre-release starts with the
/// `fork` identifier (`0.362.0-fork.42-a08b4c3111e7`), as published by the
/// fork release pipeline (work#1465).
fn is_fork_version(v: &Version) -> bool {
    v.pre.split('.').next() == Some("fork")
}

/// The numeric fork revision of a fork build (`42` in
/// `0.362.0-fork.42-a08b4c3111e7`), or `None` when absent or unparsable.
fn fork_number(v: &Version) -> Option<u64> {
    let id = v.pre.split('.').nth(1)?;
    let digits: String = id.chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<u64>().ok()
}

/// Compare a candidate release version against the running version with
/// fork awareness (work#1465).
///
/// - Both fork builds: the base version decides; on equal bases the fork number
///   decides numerically (so `fork.10` > `fork.9`). Equal fork numbers order
///   `Equal`: the sha tail on fork tags identifies the build, not a newer
///   version, and plain semver would misread a sha'd tag as older than the
///   sha-less installed version. Builds whose fork number is missing or
///   unparsable fall back to plain semver order.
/// - Every other pairing (upstream vs upstream, or the mixed fork vs upstream
///   case): plain semver order — today's behavior. Mixed pairings additionally
///   surface a warning at the call site.
fn compare_release_versions(target: &Version, current: &Version) -> std::cmp::Ordering {
    if !is_fork_version(target) || !is_fork_version(current) {
        return target.cmp(current);
    }
    let base = (target.major, target.minor, target.patch).cmp(&(
        current.major,
        current.minor,
        current.patch,
    ));
    if base != std::cmp::Ordering::Equal {
        return base;
    }
    match (fork_number(target), fork_number(current)) {
        (Some(a), Some(b)) if a != b => a.cmp(&b),
        (Some(_), Some(_)) => std::cmp::Ordering::Equal,
        _ => target.cmp(current),
    }
}

// ── Release payloads (listing + selection) ─────────────────────────────────

/// One asset attached to a release (GitHub releases JSON `assets[].name`),
/// kept so the door can resolve the version-prefixed asset names fork
/// releases attach (work#1465).
#[derive(serde::Deserialize)]
struct AssetSummary {
    name: String,
}

#[derive(serde::Deserialize)]
struct ReleaseSummary {
    tag_name: String,
    #[serde(default)]
    draft:    bool,
    /// Assets attached to the release; empty for payloads that omit them.
    #[serde(default)]
    assets:   Vec<AssetSummary>,
}

/// Pick the `tag_name` with the highest semver from `releases`, skipping
/// drafts and entries whose `tag_name` does not parse as semver. Returns
/// `None` if no candidate remains (caller may fall back to stable-latest).
fn pick_latest_tag(releases: &[ReleaseSummary]) -> Option<String> {
    releases
        .iter()
        .filter(|r| !r.draft)
        .filter_map(|r| {
            parse_version_from_tag(&r.tag_name)
                .ok()
                .map(|v| (v, &r.tag_name))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, tag)| tag.clone())
}

/// Resolve the exact asset names to download from the chosen release's
/// asset list (work#1465).
///
/// Fork releases attach version-prefixed assets
/// (`fabro-<version>-<triple>.tar.gz`) while upstream releases attach
/// version-less ones (`fabro-<triple>.tar.gz`); both end with
/// `-<triple>.tar.gz`, so a suffix match resolves the exact asset for
/// either flavor without knowing the version up front. The checksum is the
/// exact `<tarball>.sha256` sibling asset and is optional: releases may
/// ship without one (work#1465).
fn resolve_asset_names(
    tag: &str,
    assets: &[AssetSummary],
    triple: &str,
) -> Result<(String, Option<String>)> {
    let tarball_suffix = format!("-{triple}.tar.gz");
    let candidates: Vec<&str> = assets
        .iter()
        .map(|asset| asset.name.as_str())
        .filter(|name| name.ends_with(&tarball_suffix))
        .collect();
    let tarball = match candidates.as_slice() {
        [] => bail!(
            "release {tag} carries no tarball asset for {triple} (attached assets: {:?})",
            assets
                .iter()
                .map(|asset| asset.name.as_str())
                .collect::<Vec<_>>()
        ),
        [only] => (*only).to_string(),
        [..] => {
            bail!("release {tag} carries multiple tarball candidates for {triple}: {candidates:?}")
        }
    };
    let checksum_name = format!("{tarball}.sha256");
    let checksum = assets
        .iter()
        .any(|asset| asset.name == checksum_name)
        .then_some(checksum_name);
    Ok((tarball, checksum))
}

// ── SHA256 verification ────────────────────────────────────────────────────

fn verify_checksum(path: &Path, expected_hex: &str) -> Result<()> {
    let mut hasher = Sha256::new();
    let mut file = std::io::BufReader::new(
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?,
    );
    std::io::copy(&mut file, &mut hasher)
        .with_context(|| format!("reading {} for checksum", path.display()))?;
    let computed = format!("{:x}", hasher.finalize());
    // The .sha256 file may contain "hash  filename" or just "hash"
    let expected = expected_hex
        .split_whitespace()
        .next()
        .unwrap_or(expected_hex)
        .to_lowercase();
    if computed != expected {
        bail!("SHA256 mismatch: expected {expected}, got {computed}");
    }
    Ok(())
}

// ── Upgrade check state ────────────────────────────────────────────────────

const CHECK_INTERVAL_SECS: u64 = 86400; // 24 hours
const LAST_CHECK_FILE: &str = "last_upgrade_check.json";

#[derive(serde::Serialize, serde::Deserialize)]
struct UpgradeCheckState {
    checked_at:     u64,
    latest_version: String,
    #[serde(default)]
    install_source: Option<String>,
}

impl UpgradeCheckState {
    fn is_stale(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_sub(self.checked_at) >= CHECK_INTERVAL_SECS
    }

    fn is_usable_for(&self, source: InstallSource) -> bool {
        !self.is_stale() && self.install_source.as_deref() == Some(source.cache_tag())
    }

    fn load(path: &Path) -> Option<Self> {
        let data = fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        }
        let json = serde_json::to_string(self)?;
        fs::write(path, json)
            .with_context(|| format!("writing upgrade check state {}", path.display()))?;
        Ok(())
    }
}

// ── Main upgrade command ───────────────────────────────────────────────────

pub(crate) async fn run_upgrade(args: UpgradeArgs, ctx: &CommandContext) -> Result<()> {
    let cli = &ctx.user_settings().cli;
    let printer = ctx.printer();
    let current_exe = std::env::current_exe()
        .context("resolving current fabro executable path")?
        .canonicalize()
        .context("canonicalizing current fabro executable path")?;

    if let InstallSource::Brew { channel } = detect_install_source(&current_exe) {
        return run_upgrade_brew(&args, cli, printer, channel);
    }

    let backend = select_backend().await?;

    let current =
        Version::parse(env!("CARGO_PKG_VERSION")).context("failed to parse current version")?;

    // Determine target version. The release's asset list travels with the
    // selection so the download can use the exact asset names (work#1465:
    // fork releases version-prefix their assets). Fork release tags carry
    // no `v` prefix, so `--version` only prefixes one for upstream builds.
    // For `--version` the release fetch is deferred past the version gate
    // below: the "already on version" short-circuit must keep working
    // without the network.
    let (tag, version_assets) = if let Some(ref v) = args.version {
        let version = parse_version_from_tag(v)?;
        let tag = if is_fork_version(&version) {
            version.to_string()
        } else {
            format!("v{version}")
        };
        (tag, None)
    } else if args.prerelease {
        let releases = backend.fetch_releases().await?;
        if let Some(tag) = pick_latest_tag(&releases) {
            let assets = releases
                .into_iter()
                .find(|release| release.tag_name == tag)
                .map(|release| release.assets)
                .unwrap_or_default();
            (tag, Some(assets))
        } else {
            let release = backend.fetch_latest_release().await?;
            (release.tag_name, Some(release.assets))
        }
    } else {
        let release = backend.fetch_latest_release().await?;
        (release.tag_name, Some(release.assets))
    };
    let target = parse_version_from_tag(&tag)?;

    // Downgrade protection (fork-aware, work#1465)
    let mixed = is_fork_version(&target) != is_fork_version(&current);
    let ordering = compare_release_versions(&target, &current);
    if ordering == std::cmp::Ordering::Less && mixed {
        fabro_util::printerr!(
            printer,
            "Warning: comparing fork build {target} against non-fork version {current}; using \
             plain semver order (work#1465)"
        );
    }
    match ordering {
        std::cmp::Ordering::Less => {
            if args.version.is_none() {
                bail!(
                    "latest release ({target}) is older than installed version ({current}), skipping"
                );
            }
            // Explicit --version: warn + prompt
            fabro_util::printerr!(printer, "Warning: downgrading from {current} to {target}");
            if std::io::stdin().is_terminal() {
                let confirm = dialoguer::Confirm::new()
                    .with_prompt("Continue with downgrade?")
                    .default(false)
                    .interact()?;
                if !confirm {
                    bail!("downgrade cancelled");
                }
            } else {
                bail!("downgrade requires interactive confirmation (stdin is not a tty)");
            }
        }
        std::cmp::Ordering::Equal if !args.force => {
            if cli.output.format == OutputFormat::Json {
                print_json_pretty(&serde_json::json!({
                    "previous_version": current.to_string(),
                    "installed_version": current.to_string(),
                }))?;
            } else {
                fabro_util::printerr!(printer, "Already on version {current}");
            }
            return Ok(());
        }
        _ => {}
    }

    // Resolve the exact asset names for this platform before the dry-run
    // so it proves the real fork asset resolution (work#1465). The
    // `--version` path fetches the tagged release here, past the version
    // gate above.
    let release_assets = match version_assets {
        Some(assets) => assets,
        None => backend.fetch_release_by_tag(&tag).await?.assets,
    };
    let triple = detect_target()?;
    let (tarball_name, checksum_name) = resolve_asset_names(&tag, &release_assets, triple)?;

    if args.dry_run {
        if cli.output.format == OutputFormat::Json {
            print_json_pretty(&serde_json::json!({
                "previous_version": current.to_string(),
                "installed_version": target.to_string(),
                "dry_run": true,
                "tag": tag,
                "tarball_asset": tarball_name,
                "checksum_asset": checksum_name,
            }))?;
        } else {
            fabro_util::printerr!(printer, "Would upgrade fabro from {current} to {target}");
            fabro_util::printerr!(printer, "  tag: {tag}");
            fabro_util::printerr!(printer, "  target: {triple}");
            fabro_util::printerr!(printer, "  tarball: {tarball_name}");
            if let Some(ref name) = checksum_name {
                fabro_util::printerr!(printer, "  checksum: {name}");
            } else {
                fabro_util::printerr!(
                    printer,
                    "  checksum: <none attached; verification will be skipped>"
                );
            }
        }
        return Ok(());
    }

    let exe_dir = current_exe
        .parent()
        .context("could not determine executable directory")?;

    let tmp_dir = tempfile::tempdir_in(exe_dir)
        .or_else(|_| tempfile::tempdir())
        .context("failed to create temp directory")?;

    // Download tarball and checksum in parallel; a release without a
    // checksum asset upgrades unverified with a loud warning (work#1465).
    fabro_util::printerr!(printer, "Downloading fabro {target}...");
    let (tarball_path, checksum_path) = if let Some(ref checksum) = checksum_name {
        let (tarball_path, checksum_path) = tokio::try_join!(
            backend.download_release(&tag, &tarball_name, tmp_dir.path()),
            backend.download_release(&tag, checksum, tmp_dir.path()),
        )?;
        (tarball_path, Some(checksum_path))
    } else {
        fabro_util::printerr!(
            printer,
            "Warning: release {tag} ships no .sha256 checksum asset; skipping checksum \
             verification (work#1465)"
        );
        let tarball_path = backend
            .download_release(&tag, &tarball_name, tmp_dir.path())
            .await?;
        (tarball_path, None)
    };

    // Verify SHA256 using streaming hash
    if let Some(checksum_path) = checksum_path.as_ref() {
        let checksum_content = fs::read_to_string(checksum_path)
            .with_context(|| format!("reading checksum file {}", checksum_path.display()))?;
        verify_checksum(&tarball_path, &checksum_content)?;
        debug!("SHA256 checksum verified");
    }

    // Extract tarball
    let status = TokioCommand::new("tar")
        .args([
            "xzf",
            &tarball_path.to_string_lossy(),
            "-C",
            &tmp_dir.path().to_string_lossy(),
        ])
        .status()
        .await
        .context("failed to run tar")?;
    if !status.success() {
        bail!("tar extraction failed");
    }

    // Atomic binary replacement
    let extracted_binary = tmp_dir.path().join(format!("fabro-{triple}")).join("fabro");
    let backup = exe_dir.join(".fabro-upgrade-backup");
    fs::rename(&current_exe, &backup).context("failed to move current binary to backup")?;
    if let Err(e) = fs::rename(&extracted_binary, &current_exe) {
        // Restore from backup
        if let Err(restore_err) = fs::rename(&backup, &current_exe) {
            bail!(
                "failed to install new binary ({e}) and failed to restore backup ({restore_err})"
            );
        }
        bail!("failed to install new binary: {e}");
    }
    let _ = fs::remove_file(&backup);

    // Set permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&current_exe, fs::Permissions::from_mode(0o755));
    }

    if cli.output.format == OutputFormat::Json {
        print_json_pretty(&serde_json::json!({
            "previous_version": current.to_string(),
            "installed_version": target.to_string(),
        }))?;
    } else {
        fabro_util::printerr!(printer, "Upgraded fabro to {target}");
    }
    Ok(())
}

// ── Homebrew-managed upgrade path ──────────────────────────────────────────

fn run_upgrade_brew(
    args: &UpgradeArgs,
    cli: &CliNamespace,
    printer: Printer,
    channel: BrewChannel,
) -> Result<()> {
    let formula = match channel {
        BrewChannel::Stable => "fabro",
        BrewChannel::Nightly => "fabro-nightly",
    };
    let cmd = format!("brew upgrade {formula}");

    if args.version.is_some() || args.prerelease || args.force {
        bail!(
            "fabro is managed by Homebrew (formula `{formula}`); Homebrew selects the version \
             and channel. Use `brew upgrade {formula}` (or reinstall with a different formula) \
             instead of `fabro upgrade --version`/`--prerelease`/`--force`."
        );
    }

    if args.dry_run {
        if cli.output.format == OutputFormat::Json {
            print_json_pretty(&serde_json::json!({
                "install_source": "homebrew",
                "formula": formula,
                "brew_command": cmd,
                "dry_run": true,
                "note": "fabro is Homebrew-managed; no in-place upgrade attempted",
            }))?;
        } else {
            fabro_util::printerr!(printer, "fabro was installed via Homebrew.");
            fabro_util::printerr!(printer, "Run `{cmd}` to update.");
        }
        return Ok(());
    }

    fabro_util::printerr!(printer, "fabro was installed via Homebrew.");
    fabro_util::printerr!(printer, "Run `{cmd}` to update.");
    bail!("refusing to overwrite a Homebrew-managed binary");
}

// ── Auto version check ────────────────────────────────────────────────────

/// Spawn a background task that checks for a newer version and prints a notice
/// to stderr after the main command completes. Returns a handle that should be
/// awaited at the end of `main_inner`.
pub(crate) fn spawn_upgrade_check(check: bool, printer: Printer) -> Option<JoinHandle<()>> {
    if !check {
        return None;
    }
    Some(tokio::spawn(async move {
        if let Err(e) = check_and_print_notice(printer).await {
            debug!(%e, "Upgrade check failed (silently swallowed)");
        }
    }))
}

async fn check_and_print_notice(printer: Printer) -> Result<()> {
    let state_path = fabro_util::Home::from_env().root().join(LAST_CHECK_FILE);

    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;

    let current_exe = match std::env::current_exe().and_then(|p| p.canonicalize()) {
        Ok(p) => p,
        Err(e) => {
            debug!(%e, "skipping upgrade notice: cannot resolve current executable");
            return Ok(());
        }
    };
    let source = detect_install_source(&current_exe);

    if let Some(state) = UpgradeCheckState::load(&state_path) {
        if state.is_usable_for(source) {
            if let Ok(latest) = Version::parse(&state.latest_version) {
                if compare_release_versions(&latest, &current) == std::cmp::Ordering::Greater {
                    print_notice(&current, &latest, source, printer);
                }
            }
            return Ok(());
        }
    }

    let latest = match source {
        InstallSource::Brew { channel } => {
            let client = match http_client() {
                Ok(c) => c,
                Err(e) => {
                    debug!(%e, "skipping upgrade notice: failed to build HTTP client");
                    return Ok(());
                }
            };
            match fetch_tap_version(&client, channel).await {
                Ok(v) => parse_version_from_tag(&v)?,
                Err(e) => {
                    debug!(%e, "skipping upgrade notice: tap manifest fetch failed");
                    return Ok(());
                }
            }
        }
        InstallSource::Tarball => {
            let backend = select_backend().await?;
            let release = backend.fetch_latest_release().await?;
            parse_version_from_tag(&release.tag_name)?
        }
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let state = UpgradeCheckState {
        checked_at:     now,
        latest_version: latest.to_string(),
        install_source: Some(source.cache_tag().to_string()),
    };
    let _ = state.save(&state_path);

    if compare_release_versions(&latest, &current) == std::cmp::Ordering::Greater {
        print_notice(&current, &latest, source, printer);
    }

    Ok(())
}

fn print_notice(current: &Version, latest: &Version, source: InstallSource, printer: Printer) {
    fabro_util::printerr!(
        printer,
        "A new version of fabro is available: {latest} (current: {current})"
    );
    fabro_util::printerr!(printer, "Run `{}` to update.", upgrade_command_for(source));
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // -- Platform detection --

    #[test]
    fn detect_target_returns_known_triple() {
        let result = detect_target();
        // We can only assert it succeeds on known CI platforms
        if cfg!(target_os = "linux") && cfg!(target_arch = "x86_64") {
            let got = result.unwrap();
            assert!(
                got == "x86_64-unknown-linux-gnu" || got == "x86_64-unknown-linux-musl",
                "got {got}"
            );
        } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
            assert_eq!(result.unwrap(), "aarch64-apple-darwin");
        } else if cfg!(target_os = "linux") && cfg!(target_arch = "aarch64") {
            let got = result.unwrap();
            assert!(
                got == "aarch64-unknown-linux-gnu" || got == "aarch64-unknown-linux-musl",
                "got {got}"
            );
        }
        // On other platforms it would return an error, which is fine
    }

    // -- Libc parsing --

    #[test]
    fn parse_ldd_libc_detects_glibc() {
        assert_eq!(
            parse_ldd_libc(
                "ldd (Ubuntu GLIBC 2.35-0ubuntu3.4) 2.35\nCopyright (C) 2022 Free Software Foundation, Inc."
            ),
            Libc::Gnu,
        );
    }

    #[test]
    fn parse_ldd_libc_detects_musl() {
        assert_eq!(
            parse_ldd_libc("musl libc (x86_64)\nVersion 1.2.4\nDynamic Program Loader"),
            Libc::Musl,
        );
    }

    #[test]
    fn parse_ldd_libc_defaults_to_gnu_on_unknown_output() {
        assert_eq!(parse_ldd_libc(""), Libc::Gnu);
        assert_eq!(parse_ldd_libc("some unrelated output"), Libc::Gnu);
    }

    // -- Version parsing --

    #[test]
    fn parse_version_from_tag_with_v_prefix() {
        let v = parse_version_from_tag("v0.5.0").unwrap();
        assert_eq!(v, Version::new(0, 5, 0));
    }

    #[test]
    fn parse_version_from_tag_without_prefix() {
        let v = parse_version_from_tag("0.5.0").unwrap();
        assert_eq!(v, Version::new(0, 5, 0));
    }

    #[test]
    fn parse_version_from_tag_invalid() {
        assert!(parse_version_from_tag("not-a-version").is_err());
    }

    // -- SHA256 verification --

    #[test]
    fn verify_checksum_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        fs::write(&path, b"hello world").unwrap();
        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert!(verify_checksum(&path, expected).is_ok());
    }

    #[test]
    fn verify_checksum_with_filename_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        fs::write(&path, b"hello world").unwrap();
        let expected =
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9  fabro.tar.gz";
        assert!(verify_checksum(&path, expected).is_ok());
    }

    #[test]
    fn verify_checksum_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        fs::write(&path, b"hello world").unwrap();
        let wrong = "0000000000000000000000000000000000000000000000000000000000000000";
        assert!(verify_checksum(&path, wrong).is_err());
    }

    // -- Upgrade check state --

    #[test]
    fn upgrade_check_state_roundtrip() {
        let state = UpgradeCheckState {
            checked_at:     1_710_000_000,
            latest_version: "0.5.0".to_string(),
            install_source: Some("tarball".to_string()),
        };
        let json = serde_json::to_string(&state).unwrap();
        let parsed: UpgradeCheckState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.checked_at, 1_710_000_000);
        assert_eq!(parsed.latest_version, "0.5.0");
        assert_eq!(parsed.install_source.as_deref(), Some("tarball"));
    }

    #[test]
    fn upgrade_check_state_stale() {
        let old = UpgradeCheckState {
            checked_at:     0, // epoch — definitely stale
            latest_version: "0.1.0".to_string(),
            install_source: Some("tarball".to_string()),
        };
        assert!(old.is_stale());
    }

    #[test]
    fn upgrade_check_state_fresh() {
        let fresh = UpgradeCheckState {
            checked_at:     now_secs(),
            latest_version: "0.5.0".to_string(),
            install_source: Some("tarball".to_string()),
        };
        assert!(!fresh.is_stale());
    }

    #[test]
    fn upgrade_check_state_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let state = UpgradeCheckState {
            checked_at:     1_710_000_000,
            latest_version: "0.5.0".to_string(),
            install_source: Some("brew-stable".to_string()),
        };
        state.save(&path).unwrap();
        let loaded = UpgradeCheckState::load(&path).unwrap();
        assert_eq!(loaded.checked_at, 1_710_000_000);
        assert_eq!(loaded.latest_version, "0.5.0");
        assert_eq!(loaded.install_source.as_deref(), Some("brew-stable"));
    }

    // -- Backend selection --

    #[tokio::test]
    async fn select_backend_falls_back_to_http_when_gh_is_missing() {
        let backend = select_backend_for_gh_command("fabro-test-gh-that-should-not-exist")
            .await
            .unwrap();
        assert!(matches!(backend, Backend::Http(_)));
    }

    // -- Release selection --

    fn release(tag: &str, draft: bool) -> ReleaseSummary {
        ReleaseSummary {
            tag_name: tag.to_string(),
            draft,
            assets: Vec::new(),
        }
    }

    #[test]
    fn pick_latest_tag_prefers_newer_prerelease_when_stable_is_older() {
        let releases = [
            release("v0.204.0-beta.0", false),
            release("v0.203.0", false),
            release("v0.202.0", false),
        ];
        assert_eq!(
            pick_latest_tag(&releases).as_deref(),
            Some("v0.204.0-beta.0")
        );
    }

    #[test]
    fn pick_latest_tag_prefers_newer_stable_over_older_prerelease() {
        let releases = [
            release("v0.205.0", false),
            release("v0.205.0-beta.1", false),
            release("v0.204.0", false),
        ];
        assert_eq!(pick_latest_tag(&releases).as_deref(), Some("v0.205.0"));
    }

    #[test]
    fn pick_latest_tag_filters_drafts() {
        let releases = [
            release("v0.300.0", true),
            release("v0.204.0-beta.0", false),
            release("v0.203.0", false),
        ];
        assert_eq!(
            pick_latest_tag(&releases).as_deref(),
            Some("v0.204.0-beta.0")
        );
    }

    #[test]
    fn pick_latest_tag_skips_unparseable_tags() {
        let releases = [
            release("weekly-build-3", false),
            release("v0.204.0-beta.0", false),
        ];
        assert_eq!(
            pick_latest_tag(&releases).as_deref(),
            Some("v0.204.0-beta.0")
        );
    }

    #[test]
    fn pick_latest_tag_returns_none_when_all_drafts() {
        let releases = [release("v0.300.0", true), release("v0.299.0", true)];
        assert_eq!(pick_latest_tag(&releases), None);
    }

    #[test]
    fn pick_latest_tag_returns_none_for_empty_input() {
        assert_eq!(pick_latest_tag(&[]), None);
    }

    // -- Install source detection --

    #[test]
    fn detect_install_source_macos_arm_cellar() {
        let p = PathBuf::from("/opt/homebrew/Cellar/fabro/0.176.2/bin/fabro");
        assert_eq!(detect_install_source(&p), InstallSource::Brew {
            channel: BrewChannel::Stable,
        });
    }

    #[test]
    fn detect_install_source_macos_intel_cellar() {
        let p = PathBuf::from("/usr/local/Cellar/fabro/0.176.2/bin/fabro");
        assert_eq!(detect_install_source(&p), InstallSource::Brew {
            channel: BrewChannel::Stable,
        });
    }

    #[test]
    fn detect_install_source_linuxbrew() {
        let p = PathBuf::from("/home/linuxbrew/.linuxbrew/Cellar/fabro/0.176.2/bin/fabro");
        assert_eq!(detect_install_source(&p), InstallSource::Brew {
            channel: BrewChannel::Stable,
        });
    }

    #[test]
    fn detect_install_source_nightly() {
        let p = PathBuf::from("/opt/homebrew/Cellar/fabro-nightly/0.205.0-nightly.0/bin/fabro");
        assert_eq!(detect_install_source(&p), InstallSource::Brew {
            channel: BrewChannel::Nightly,
        });
    }

    #[test]
    fn detect_install_source_tarball() {
        let p = PathBuf::from("/usr/local/bin/fabro");
        assert_eq!(detect_install_source(&p), InstallSource::Tarball);
    }

    #[test]
    fn detect_install_source_home() {
        let p = PathBuf::from("/Users/foo/.fabro/bin/fabro");
        assert_eq!(detect_install_source(&p), InstallSource::Tarball);
    }

    // -- Tap versions.json parsing --

    const TAP_FIXTURE: &str = r#"{
        "stable":  { "version": "0.204.0",           "tag": "v0.204.0" },
        "nightly": { "version": "0.205.0-nightly.0", "tag": "v0.205.0-nightly.0" }
    }"#;

    #[test]
    fn parse_tap_versions_stable() {
        assert_eq!(
            parse_tap_versions(TAP_FIXTURE, BrewChannel::Stable).unwrap(),
            "0.204.0"
        );
    }

    #[test]
    fn parse_tap_versions_nightly() {
        assert_eq!(
            parse_tap_versions(TAP_FIXTURE, BrewChannel::Nightly).unwrap(),
            "0.205.0-nightly.0"
        );
    }

    #[test]
    fn parse_tap_versions_missing_channel_errors() {
        let body = r#"{ "stable": { "version": "0.1.0" } }"#;
        assert!(parse_tap_versions(body, BrewChannel::Nightly).is_err());
    }

    #[test]
    fn parse_tap_versions_malformed_errors() {
        assert!(parse_tap_versions("not json", BrewChannel::Stable).is_err());
    }

    // -- Source-aware cache --

    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn cache_is_usable_when_source_matches() {
        let state = UpgradeCheckState {
            checked_at:     now_secs(),
            latest_version: "0.5.0".into(),
            install_source: Some("brew-stable".into()),
        };
        assert!(state.is_usable_for(InstallSource::Brew {
            channel: BrewChannel::Stable,
        }));
    }

    #[test]
    fn cache_ignored_when_channel_differs() {
        let state = UpgradeCheckState {
            checked_at:     now_secs(),
            latest_version: "0.5.0".into(),
            install_source: Some("brew-stable".into()),
        };
        assert!(!state.is_usable_for(InstallSource::Brew {
            channel: BrewChannel::Nightly,
        }));
    }

    #[test]
    fn cache_ignored_when_source_differs() {
        let state = UpgradeCheckState {
            checked_at:     now_secs(),
            latest_version: "0.5.0".into(),
            install_source: Some("tarball".into()),
        };
        assert!(!state.is_usable_for(InstallSource::Brew {
            channel: BrewChannel::Stable,
        }));
    }

    #[test]
    fn legacy_cache_without_install_source_is_not_usable() {
        let state = UpgradeCheckState {
            checked_at:     now_secs(),
            latest_version: "0.5.0".into(),
            install_source: None,
        };
        assert!(!state.is_usable_for(InstallSource::Tarball));
        assert!(!state.is_usable_for(InstallSource::Brew {
            channel: BrewChannel::Stable,
        }));
    }

    #[test]
    fn legacy_cache_json_deserializes_with_none_source() {
        // Old payload shape — no install_source field.
        let json = r#"{"checked_at":1710000000,"latest_version":"0.5.0"}"#;
        let state: UpgradeCheckState = serde_json::from_str(json).unwrap();
        assert_eq!(state.install_source, None);
    }

    // -- Notice command selection --

    #[test]
    fn upgrade_command_tarball() {
        assert_eq!(upgrade_command_for(InstallSource::Tarball), "fabro upgrade");
    }

    #[test]
    fn upgrade_command_brew_stable() {
        assert_eq!(
            upgrade_command_for(InstallSource::Brew {
                channel: BrewChannel::Stable,
            }),
            "brew upgrade fabro"
        );
    }

    #[test]
    fn upgrade_command_brew_nightly() {
        assert_eq!(
            upgrade_command_for(InstallSource::Brew {
                channel: BrewChannel::Nightly,
            }),
            "brew upgrade fabro-nightly"
        );
    }

    // -- Brew-managed run_upgrade path --

    fn brew_args(
        version: Option<&str>,
        prerelease: bool,
        force: bool,
        dry_run: bool,
    ) -> UpgradeArgs {
        UpgradeArgs {
            version: version.map(str::to_string),
            prerelease,
            force,
            dry_run,
        }
    }

    #[test]
    fn run_upgrade_brew_refuses_by_default() {
        let cli = CliNamespace::default();
        let err = run_upgrade_brew(
            &brew_args(None, false, false, false),
            &cli,
            Printer::Silent,
            BrewChannel::Stable,
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("Homebrew"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn run_upgrade_brew_dry_run_returns_ok() {
        let cli = CliNamespace::default();
        let result = run_upgrade_brew(
            &brew_args(None, false, false, true),
            &cli,
            Printer::Silent,
            BrewChannel::Nightly,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn run_upgrade_brew_rejects_version_flag() {
        let cli = CliNamespace::default();
        let err = run_upgrade_brew(
            &brew_args(Some("0.1.0"), false, false, false),
            &cli,
            Printer::Silent,
            BrewChannel::Stable,
        )
        .unwrap_err();
        assert!(err.to_string().contains("Homebrew"));
    }

    #[test]
    fn run_upgrade_brew_rejects_prerelease_flag() {
        let cli = CliNamespace::default();
        let err = run_upgrade_brew(
            &brew_args(None, true, false, false),
            &cli,
            Printer::Silent,
            BrewChannel::Stable,
        )
        .unwrap_err();
        assert!(err.to_string().contains("Homebrew"));
    }

    #[test]
    fn run_upgrade_brew_rejects_force_flag() {
        let cli = CliNamespace::default();
        let err = run_upgrade_brew(
            &brew_args(None, false, true, false),
            &cli,
            Printer::Silent,
            BrewChannel::Stable,
        )
        .unwrap_err();
        assert!(err.to_string().contains("Homebrew"));
    }

    // -- Fork-aware repo selection (work#1465) --

    #[test]
    fn release_repo_targets_fork_repo_for_fork_builds() {
        // The selection rule is compile-time: fork builds (version carries
        // `-fork`) target the fork repo, upstream builds stay upstream.
        let expected = if env!("CARGO_PKG_VERSION").contains("-fork") {
            FORK_GITHUB_REPO
        } else {
            GITHUB_REPO
        };
        assert_eq!(release_repo(), expected);
        // Pin the two repos themselves so a rename cannot slip silently.
        assert_eq!(FORK_GITHUB_REPO, "denkhaus/fabro");
        assert_eq!(GITHUB_REPO, "fabro-sh/fabro");
    }

    // -- Fork version parsing (work#1465) --

    #[test]
    fn is_fork_version_detects_fork_prerelease() {
        assert!(is_fork_version(&Version::parse("0.362.0-fork.42").unwrap()));
        assert!(is_fork_version(
            &Version::parse("0.362.0-fork.42-a08b4c3111e7").unwrap()
        ));
        assert!(!is_fork_version(&Version::parse("0.362.0").unwrap()));
        assert!(!is_fork_version(
            &Version::parse("0.205.0-nightly.0").unwrap()
        ));
    }

    #[test]
    fn fork_number_extracts_numeric_revision() {
        assert_eq!(
            fork_number(&Version::parse("0.362.0-fork.42").unwrap()),
            Some(42)
        );
        assert_eq!(
            fork_number(&Version::parse("0.362.0-fork.10-a08b4c3111e7").unwrap()),
            Some(10)
        );
        assert_eq!(fork_number(&Version::parse("0.362.0-fork").unwrap()), None);
        assert_eq!(fork_number(&Version::parse("0.362.0").unwrap()), None);
    }

    // -- Fork-aware version comparison (work#1465) --

    #[test]
    fn compare_orders_fork_numbers_numerically() {
        let newer = Version::parse("0.362.0-fork.10").unwrap();
        let older = Version::parse("0.362.0-fork.9").unwrap();
        assert_eq!(
            compare_release_versions(&newer, &older),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_release_versions(&older, &newer),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn compare_equal_fork_numbers_with_sha_tail_is_equal() {
        // The sha tail identifies the build, not a newer version; plain
        // semver would order the sha'd tag as older than the sha-less one.
        let tagged = Version::parse("0.362.0-fork.42-a08b4c3111e7").unwrap();
        let installed = Version::parse("0.362.0-fork.42").unwrap();
        assert_eq!(
            compare_release_versions(&tagged, &installed),
            std::cmp::Ordering::Equal
        );
        assert_eq!(
            compare_release_versions(&installed, &tagged),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn compare_orders_fork_builds_by_base_version_first() {
        let newer_base = Version::parse("0.363.0-fork.1").unwrap();
        let older_base = Version::parse("0.362.0-fork.99").unwrap();
        assert_eq!(
            compare_release_versions(&newer_base, &older_base),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn compare_mixed_pairs_keep_plain_semver_order() {
        let fork = Version::parse("0.362.0-fork.42").unwrap();
        let upstream = Version::parse("0.361.0").unwrap();
        assert_eq!(
            compare_release_versions(&fork, &upstream),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_release_versions(&upstream, &fork),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn compare_unparsable_fork_number_falls_back_to_plain_semver() {
        let shaless = Version::parse("0.362.0-fork").unwrap();
        let numbered = Version::parse("0.362.0-fork.1").unwrap();
        // Plain semver: `fork.1` pre-release sorts after bare `fork`.
        assert_eq!(
            compare_release_versions(&numbered, &shaless),
            numbered.cmp(&shaless)
        );
    }

    // -- Asset-name resolution (work#1465) --

    fn assets(names: &[&str]) -> Vec<AssetSummary> {
        names
            .iter()
            .map(|name| AssetSummary {
                name: (*name).to_string(),
            })
            .collect()
    }

    const TEST_TRIPLE: &str = "x86_64-unknown-linux-gnu";

    #[test]
    fn resolve_asset_names_matches_fork_version_prefixed_tarball() {
        let list = assets(&[
            "fabro-0.362.0-fork.42-x86_64-unknown-linux-gnu.tar.gz",
            "fabro-0.362.0-fork.42-x86_64-unknown-linux-gnu.tar.gz.sha256",
            "fabro-0.362.0-fork.42-aarch64-apple-darwin.tar.gz",
        ]);
        let (tarball, checksum) =
            resolve_asset_names("0.362.0-fork.42-a08b4c3111e7", &list, TEST_TRIPLE).unwrap();
        assert_eq!(
            tarball,
            "fabro-0.362.0-fork.42-x86_64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(
            checksum.as_deref(),
            Some("fabro-0.362.0-fork.42-x86_64-unknown-linux-gnu.tar.gz.sha256")
        );
    }

    #[test]
    fn resolve_asset_names_matches_upstream_version_less_tarball() {
        let list = assets(&[
            "fabro-x86_64-unknown-linux-gnu.tar.gz",
            "fabro-x86_64-unknown-linux-gnu.tar.gz.sha256",
        ]);
        let (tarball, checksum) = resolve_asset_names("v0.254.0", &list, TEST_TRIPLE).unwrap();
        assert_eq!(tarball, "fabro-x86_64-unknown-linux-gnu.tar.gz");
        assert_eq!(
            checksum.as_deref(),
            Some("fabro-x86_64-unknown-linux-gnu.tar.gz.sha256")
        );
    }

    #[test]
    fn resolve_asset_names_returns_none_checksum_when_sibling_absent() {
        let list = assets(&["fabro-0.362.0-fork.41-x86_64-unknown-linux-gnu.tar.gz"]);
        let (tarball, checksum) = resolve_asset_names("tag", &list, TEST_TRIPLE).unwrap();
        assert_eq!(
            tarball,
            "fabro-0.362.0-fork.41-x86_64-unknown-linux-gnu.tar.gz"
        );
        assert_eq!(checksum, None);
    }

    #[test]
    fn resolve_asset_names_errors_when_no_tarball_for_triple() {
        let list = assets(&["fabro-0.362.0-fork.42-aarch64-apple-darwin.tar.gz"]);
        let err = resolve_asset_names("tag", &list, TEST_TRIPLE).unwrap_err();
        assert!(
            err.to_string().contains("no tarball asset"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn resolve_asset_names_errors_on_multiple_tarball_candidates() {
        let list = assets(&[
            "fabro-0.362.0-fork.42-x86_64-unknown-linux-gnu.tar.gz",
            "fabro-0.361.0-fork.41-x86_64-unknown-linux-gnu.tar.gz",
        ]);
        let err = resolve_asset_names("tag", &list, TEST_TRIPLE).unwrap_err();
        assert!(
            err.to_string().contains("multiple tarball candidates"),
            "unexpected error: {err}"
        );
    }
}
