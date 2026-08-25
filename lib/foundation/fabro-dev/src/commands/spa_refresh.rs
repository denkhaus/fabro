use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::Args;
use walkdir::WalkDir;

use super::spa_check::check_spa_asset_budgets;
use super::workspace_root;

const DEFAULT_ASSET_BUDGET_BYTES: u64 = 15 * 1024 * 1024;
const DEFAULT_PAYLOAD_BUDGET_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Args)]
pub(crate) struct SpaRefreshArgs {
    /// Repository root containing apps/fabro-web and lib/apps/fabro-spa.
    #[arg(long, hide = true)]
    root: Option<PathBuf>,
    /// Override the raw asset budget.
    #[arg(long, hide = true, default_value_t = DEFAULT_ASSET_BUDGET_BYTES)]
    pub(super) asset_budget_bytes: u64,
    /// Override the estimated gzip payload budget.
    #[arg(long, hide = true, default_value_t = DEFAULT_PAYLOAD_BUDGET_BYTES)]
    pub(super) payload_budget_bytes: u64,
}

pub(crate) fn spa_refresh(args: SpaRefreshArgs) -> Result<()> {
    let root = args.root.unwrap_or_else(workspace_root);
    spa_refresh_root_with_budgets(&root, args.asset_budget_bytes, args.payload_budget_bytes)
}

pub(crate) fn spa_refresh_root(root: &Path) -> Result<()> {
    spa_refresh_root_with_budgets(
        root,
        DEFAULT_ASSET_BUDGET_BYTES,
        DEFAULT_PAYLOAD_BUDGET_BYTES,
    )
}

#[expect(
    clippy::print_stdout,
    reason = "dev spa refresh command reports progress directly"
)]
fn spa_refresh_root_with_budgets(
    root: &Path,
    asset_budget_bytes: u64,
    payload_budget_bytes: u64,
) -> Result<()> {
    let web_dir = root.join("apps/fabro-web");
    let dist_dir = web_dir.join("dist");
    let asset_dir = root.join("lib/apps/fabro-spa/assets");

    let _lock = SpaRefreshLock::acquire(root)?;
    println!("Running bun run build in apps/fabro-web...");
    run_bun_build(&web_dir)?;

    refresh_from_dist(
        root,
        &dist_dir,
        &asset_dir,
        asset_budget_bytes,
        payload_budget_bytes,
    )?;
    println!("Refreshed lib/apps/fabro-spa/assets");

    Ok(())
}

fn refresh_from_dist(
    root: &Path,
    dist_dir: &Path,
    asset_dir: &Path,
    asset_budget_bytes: u64,
    payload_budget_bytes: u64,
) -> Result<()> {
    let staging = TempDir::new(root, "refresh")?;
    mirror_dist(dist_dir, staging.path())?;
    check_spa_asset_budgets(staging.path(), asset_budget_bytes, payload_budget_bytes)?;
    // Refuse a mixed snapshot (dist rewritten mid-walk: new index.html, missing
    // entry assets) BEFORE touching the embedded dir — the failure mode from
    // fabro-332e shipped an index.html without any hashed assets.
    verify_index_references(staging.path())?;
    publish_staging(staging.path(), asset_dir)?;

    Ok(())
}

/// Publish a verified staging snapshot into the embedded assets dir.
///
/// Ordering contract (crash safety): additive copies first, the `index.html`
/// flip LAST, stale removal only after the flip. `index.html` is the only
/// file referencing hashed assets by name, so a crash at any point leaves the
/// embedded dir rendering the OLD or the NEW bundle — never an index.html
/// pointing at assets that are not there. Every file is replaced atomically
/// (temp file + rename), so a crash or full disk mid-copy cannot leave a
/// truncated file under a final name. Temp files that survive a crash are
/// removed by the next refresh's stale sweep.
fn publish_staging(staging: &Path, asset_dir: &Path) -> Result<()> {
    let plan = mirror_plan(staging)?;
    if !plan
        .files
        .iter()
        .any(|file| file.relative == Path::new("index.html"))
    {
        bail!(
            "staging snapshot has no index.html; refusing to touch {}",
            asset_dir.display()
        );
    }

    std::fs::create_dir_all(asset_dir)
        .with_context(|| format!("creating {}", asset_dir.display()))?;
    for relative_dir in &plan.dirs {
        let destination = asset_dir.join(relative_dir);
        std::fs::create_dir_all(&destination)
            .with_context(|| format!("creating {}", destination.display()))?;
    }

    let index_marker = Path::new("index.html");
    for source_file in &plan.files {
        if source_file.relative == index_marker {
            continue;
        }
        copy_if_changed(&source_file.source, &asset_dir.join(&source_file.relative))?;
    }
    write_if_changed(&asset_dir.join(".gitkeep"), b"")?;

    // Flip last.
    let index = plan
        .files
        .iter()
        .find(|file| file.relative == index_marker)
        .expect("index.html presence checked above");
    copy_if_changed(&index.source, &asset_dir.join(index_marker))?;

    remove_stale_entries(asset_dir, &plan)?;

    // Post-condition: never compile a bundle whose index references missing
    // assets (final guard for `cargo dev build` / `docker-build`).
    verify_index_references(asset_dir)
}

/// Verify every root-relative `src=` / `href=` reference in `index.html`
/// resolves to a file under `asset_dir`.
#[expect(
    clippy::disallowed_methods,
    reason = "dev spa refresh verifies the bundle with a synchronous index.html read"
)]
fn verify_index_references(asset_dir: &Path) -> Result<()> {
    let index_path = asset_dir.join("index.html");
    let html = std::fs::read_to_string(&index_path)
        .with_context(|| format!("reading {}", index_path.display()))?;
    let missing = extract_root_relative_references(&html)
        .into_iter()
        .filter(|reference| !asset_dir.join(reference).is_file())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }

    bail!(
        "index.html references {} missing asset(s): [{}]; the SPA bundle is          inconsistent (dist rewritten mid-mirror?) — refusing to publish/compile",
        missing.len(),
        missing.join(", ")
    );
}

/// Root-relative, same-origin references (`/assets/app-abc123.js`) from
/// `src=` / `href=` attributes; absolute URLs, protocol-relative URLs,
/// inline data, and bare `/` are ignored. Query strings and fragments are
/// stripped.
fn extract_root_relative_references(html: &str) -> Vec<String> {
    let mut references = BTreeSet::new();
    for marker in ["src=", "href="] {
        let mut cursor = 0usize;
        while let Some(offset) = html[cursor..].find(marker) {
            let value_start = cursor + offset + marker.len();
            cursor = value_start;
            let Some(quote) = html[value_start..]
                .chars()
                .next()
                .filter(|c| *c == '"' || *c == '\'')
            else {
                continue;
            };
            let rest = &html[value_start + 1..];
            let Some(end) = rest.find(quote) else {
                break;
            };
            cursor = value_start + 1 + end;
            let value = &rest[..end];
            if let Some(reference) = root_relative_target(value) {
                references.insert(reference);
            }
        }
    }
    references.into_iter().collect()
}

/// Normalize an attribute value to a repository-relative target, if it is a
/// root-relative same-origin path.
fn root_relative_target(value: &str) -> Option<String> {
    if !value.starts_with('/') || value.starts_with("//") {
        return None;
    }
    let clean = value.split(['?', '#']).next().unwrap_or(value).trim();
    if clean.len() <= 1 {
        return None;
    }
    Some(clean[1..].to_string())
}

#[expect(
    clippy::disallowed_methods,
    reason = "dev spa refresh intentionally runs a synchronous Bun subprocess"
)]
pub(super) fn run_bun_build(web_dir: &Path) -> Result<()> {
    let status = Command::new("bun")
        .args(["run", "build"])
        .current_dir(web_dir)
        .status()
        .with_context(|| format!("running bun run build in {}", web_dir.display()))?;
    if !status.success() {
        bail!("bun run build failed with {status}");
    }

    Ok(())
}

pub(super) fn mirror_dist(dist_dir: &Path, asset_dir: &Path) -> Result<()> {
    if !dist_dir.is_dir() {
        bail!("apps/fabro-web/dist is missing; run `bun run build` before mirroring SPA assets");
    }

    let plan = mirror_plan(dist_dir)?;
    remove_stale_entries(asset_dir, &plan)?;
    std::fs::create_dir_all(asset_dir)
        .with_context(|| format!("creating {}", asset_dir.display()))?;

    for relative_dir in &plan.dirs {
        let destination = asset_dir.join(relative_dir);
        std::fs::create_dir_all(&destination)
            .with_context(|| format!("creating {}", destination.display()))?;
    }

    for source_file in &plan.files {
        copy_if_changed(&source_file.source, &asset_dir.join(&source_file.relative))?;
    }

    write_if_changed(&asset_dir.join(".gitkeep"), b"")?;

    Ok(())
}

struct MirrorPlan {
    dirs:  BTreeSet<PathBuf>,
    files: Vec<SourceFile>,
}

struct SourceFile {
    source:   PathBuf,
    relative: PathBuf,
}

fn mirror_plan(dist_dir: &Path) -> Result<MirrorPlan> {
    let mut dirs = BTreeSet::new();
    let mut files = Vec::new();

    for entry in WalkDir::new(dist_dir) {
        let entry = entry.context("walking apps/fabro-web/dist")?;
        let source = entry.path();
        let relative = source
            .strip_prefix(dist_dir)
            .with_context(|| format!("{} is not under {}", source.display(), dist_dir.display()))?;
        if relative.as_os_str().is_empty() {
            continue;
        }

        if entry.file_type().is_dir() {
            dirs.insert(relative.to_path_buf());
            continue;
        }

        if source.extension().and_then(|ext| ext.to_str()) == Some("map") {
            continue;
        }

        for ancestor in relative.ancestors().skip(1) {
            if ancestor.as_os_str().is_empty() {
                break;
            }
            dirs.insert(ancestor.to_path_buf());
        }

        files.push(SourceFile {
            source:   source.to_path_buf(),
            relative: relative.to_path_buf(),
        });
    }

    files.sort_by(|left, right| left.relative.cmp(&right.relative));

    Ok(MirrorPlan { dirs, files })
}

fn remove_stale_entries(asset_dir: &Path, plan: &MirrorPlan) -> Result<()> {
    if !asset_dir.exists() {
        return Ok(());
    }

    let desired_files = plan
        .files
        .iter()
        .map(|source| source.relative.clone())
        .chain([PathBuf::from(".gitkeep")])
        .collect::<BTreeSet<_>>();

    for entry in WalkDir::new(asset_dir).contents_first(true) {
        let entry = entry.context("walking fabro-spa assets")?;
        let path = entry.path();
        let relative = path
            .strip_prefix(asset_dir)
            .with_context(|| format!("{} is not under {}", path.display(), asset_dir.display()))?;
        if relative.as_os_str().is_empty() {
            continue;
        }

        if entry.file_type().is_dir() {
            if !plan.dirs.contains(relative) {
                std::fs::remove_dir(path)
                    .with_context(|| format!("removing {}", path.display()))?;
            }
        } else if !desired_files.contains(relative) {
            std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
        }
    }

    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "dev spa refresh mirrors build output with synchronous filesystem operations"
)]
fn copy_if_changed(source: &Path, destination: &Path) -> Result<()> {
    if destination.is_file() && files_match(source, destination)? {
        return Ok(());
    }

    let parent = destination.parent().unwrap_or(destination);
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let file_name = destination.file_name().map_or_else(
        || "asset".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let temp = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));
    std::fs::copy(source, &temp)
        .with_context(|| format!("copying {} to {}", source.display(), temp.display()))?;
    // Atomic replace: readers (and rust-embed) never observe a truncated
    // file under the final name, even if the copy died on a full disk.
    std::fs::rename(&temp, destination).with_context(|| {
        format!(
            "replacing {} with {}",
            destination.display(),
            temp.display()
        )
    })?;
    Ok(())
}

#[expect(
    clippy::disallowed_methods,
    reason = "dev spa refresh writes marker files synchronously while mirroring build output"
)]
fn write_if_changed(destination: &Path, contents: &[u8]) -> Result<()> {
    if destination.is_file()
        && std::fs::read(destination)
            .with_context(|| format!("reading {}", destination.display()))?
            == contents
    {
        return Ok(());
    }

    let parent = destination.parent().unwrap_or(destination);
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let file_name = destination.file_name().map_or_else(
        || "asset".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let temp = parent.join(format!(".{file_name}.tmp-{}", std::process::id()));
    std::fs::write(&temp, contents).with_context(|| format!("writing {}", temp.display()))?;
    std::fs::rename(&temp, destination)
        .with_context(|| format!("replacing {}", destination.display()))
}

#[expect(
    clippy::disallowed_methods,
    reason = "dev spa refresh compares generated asset bytes synchronously before mirroring"
)]
fn files_match(left: &Path, right: &Path) -> Result<bool> {
    let left_len = left
        .metadata()
        .with_context(|| format!("reading metadata for {}", left.display()))?
        .len();
    let right_len = right
        .metadata()
        .with_context(|| format!("reading metadata for {}", right.display()))?
        .len();
    if left_len != right_len {
        return Ok(false);
    }

    Ok(
        std::fs::read(left).with_context(|| format!("reading {}", left.display()))?
            == std::fs::read(right).with_context(|| format!("reading {}", right.display()))?,
    )
}

/// Cross-process lock serializing spa refresh/check (bun build + mirror +
/// publish). Two concurrent mirrors racing on the shared `dist/` directory
/// and the embedded assets dir produced the broken bundle in fabro-332e;
/// the `just up` flock covers the pipeline, this lock covers direct
/// `cargo dev build` / `cargo dev docker-build` invocations.
#[derive(Debug)]
pub(super) struct SpaRefreshLock {
    path: PathBuf,
}

impl SpaRefreshLock {
    #[expect(
        clippy::disallowed_methods,
        reason = "spa lock uses a synchronous create_new open for cross-process exclusion"
    )]
    #[expect(
        clippy::disallowed_types,
        reason = "spa lock writes the holder pid with a synchronous writer"
    )]
    pub(super) fn acquire(root: &Path) -> Result<Self> {
        let dir = root.join("tmp");
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join("fabro-dev-spa.lock");
        for attempt in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    use std::io::Write as _;
                    let pid = std::process::id();
                    let _ = writeln!(file, "{pid}");
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if attempt == 0 && lock_is_stale(&path)? {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    let holder = std::fs::read_to_string(&path).map_or_else(
                        |_| "<unreadable>".to_string(),
                        |contents| contents.trim().to_string(),
                    );
                    bail!(
                        "another spa refresh/check is running (pid {holder}); if this is stale,                          remove {}",
                        path.display()
                    );
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("creating {}", path.display()));
                }
            }
        }
        bail!("could not acquire spa lock at {}", path.display());
    }
}

impl Drop for SpaRefreshLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Stale detection: the recorded pid is gone (Linux `/proc`), or the lock
/// file is older than the staleness window (any platform).
#[expect(
    clippy::disallowed_methods,
    reason = "dev spa lock staleness check uses synchronous filesystem inspection"
)]
fn lock_is_stale(path: &Path) -> Result<bool> {
    let stale_after = chrono::Duration::minutes(30);

    let contents =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if let Ok(pid) = contents.trim().parse::<u32>() {
        if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
            return Ok(true);
        }
    }
    let modified: chrono::DateTime<chrono::Utc> = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(anyhow::Error::from)
        .map(Into::into)
        .with_context(|| format!("reading mtime for {}", path.display()))?;
    Ok(chrono::Utc::now().signed_duration_since(modified) > stale_after)
}

pub(super) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub(super) fn new(root: &Path, label: &str) -> Result<Self> {
        let base = root.join("tmp");
        std::fs::create_dir_all(&base).with_context(|| format!("creating {}", base.display()))?;

        for attempt in 0..100 {
            let path = base.join(format!(
                "fabro-dev-spa-{label}-{}-{attempt}",
                std::process::id()
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(error).with_context(|| format!("creating {}", path.display()));
                }
            }
        }

        bail!(
            "could not create temporary SPA staging directory under {}",
            base.display()
        )
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "tests stage temporary SPA fixture files with sync std::fs operations"
)]
mod tests {
    use std::path::Path;

    use super::{
        SpaRefreshLock, extract_root_relative_references, mirror_dist, publish_staging,
        refresh_from_dist,
    };

    fn write_file(root: &Path, path: &str, contents: impl AsRef<[u8]>) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("fixture path should have parent"))
            .expect("creating fixture parent directory");
        std::fs::write(path, contents).expect("writing fixture file");
    }

    fn read_bytes(root: &Path, path: &str) -> Vec<u8> {
        std::fs::read(root.join(path)).expect("reading fixture file")
    }

    #[cfg(unix)]
    #[test]
    fn mirror_dist_preserves_unchanged_assets() {
        use std::os::unix::fs::MetadataExt;

        let fixture = tempfile::tempdir().expect("creating fixture");
        write_file(fixture.path(), "dist/index.html", b"index");
        write_file(fixture.path(), "assets/index.html", b"index");

        let before_inode = std::fs::metadata(fixture.path().join("assets/index.html"))
            .expect("reading initial metadata")
            .ino();

        mirror_dist(&fixture.path().join("dist"), &fixture.path().join("assets"))
            .expect("mirroring dist");

        let after_inode = std::fs::metadata(fixture.path().join("assets/index.html"))
            .expect("reading mirrored metadata")
            .ino();

        assert_eq!(
            before_inode, after_inode,
            "unchanged asset files should not be deleted and recreated"
        );
    }

    #[test]
    fn mirror_dist_removes_stale_files_source_maps_and_keeps_directory_tracked() {
        let fixture = tempfile::tempdir().expect("creating fixture");
        write_file(fixture.path(), "dist/index.html", b"index");
        write_file(fixture.path(), "dist/assets/app.js", b"app");
        write_file(fixture.path(), "dist/assets/app.js.map", b"map");
        write_file(fixture.path(), "assets/stale.txt", b"stale");

        mirror_dist(&fixture.path().join("dist"), &fixture.path().join("assets"))
            .expect("mirroring dist");

        assert!(fixture.path().join("assets/index.html").is_file());
        assert!(fixture.path().join("assets/assets/app.js").is_file());
        assert!(fixture.path().join("assets/.gitkeep").is_file());
        assert!(!fixture.path().join("assets/assets/app.js.map").exists());
        assert!(!fixture.path().join("assets/stale.txt").exists());
    }

    #[test]
    fn mirror_dist_missing_source_errors_cleanly() {
        let fixture = tempfile::tempdir().expect("creating fixture");

        let error = mirror_dist(&fixture.path().join("dist"), &fixture.path().join("assets"))
            .expect_err("missing dist should fail");

        assert!(
            error
                .to_string()
                .contains("apps/fabro-web/dist is missing; run `bun run build`"),
            "missing dist should explain how to recover: {error:#}"
        );
    }

    #[test]
    fn refresh_budget_failure_leaves_assets_untouched() {
        let fixture = tempfile::tempdir().expect("creating fixture");
        write_file(fixture.path(), "apps/fabro-web/dist/index.html", b"hello");
        write_file(
            fixture.path(),
            "lib/apps/fabro-spa/assets/index.html",
            b"embedded",
        );

        let error = refresh_from_dist(
            fixture.path(),
            &fixture.path().join("apps/fabro-web/dist"),
            &fixture.path().join("lib/apps/fabro-spa/assets"),
            4,
            100,
        )
        .expect_err("budget failure should fail");

        assert!(
            error
                .to_string()
                .contains("fabro-spa embedded assets exceed budget: 5 > 4"),
            "budget failure should report raw byte overage: {error:#}"
        );
        assert_eq!(
            read_bytes(fixture.path(), "lib/apps/fabro-spa/assets/index.html"),
            b"embedded"
        );
    }

    #[test]
    fn refresh_mixed_snapshot_refuses_before_touching_embedded_dir() {
        // The fabro-332e failure shape: dist walked mid-rewrite, index.html
        // already references the NEW entry asset while the walk only saw the
        // old one. The snapshot must be refused BEFORE the embedded dir
        // loses anything.
        let fixture = tempfile::tempdir().expect("creating fixture");
        write_file(
            fixture.path(),
            "apps/fabro-web/dist/index.html",
            br#"<html><script src="/assets/entry-pw97apn6.js"></script></html>"#,
        );
        write_file(
            fixture.path(),
            "apps/fabro-web/dist/assets/entry-34ptkwsq.js",
            b"other build",
        );
        write_file(
            fixture.path(),
            "lib/apps/fabro-spa/assets/index.html",
            br#"<html><script src="/assets/entry-old.js"></script></html>"#,
        );
        write_file(
            fixture.path(),
            "lib/apps/fabro-spa/assets/assets/entry-old.js",
            b"old",
        );

        let error = refresh_from_dist(
            fixture.path(),
            &fixture.path().join("apps/fabro-web/dist"),
            &fixture.path().join("lib/apps/fabro-spa/assets"),
            1_000_000,
            1_000_000,
        )
        .expect_err("mixed snapshot should be refused");

        assert!(
            error.to_string().contains("missing asset(s)"),
            "mixed snapshot should name the missing reference: {error:#}"
        );
        assert_eq!(
            read_bytes(fixture.path(), "lib/apps/fabro-spa/assets/index.html"),
            br#"<html><script src="/assets/entry-old.js"></script></html>"#,
            "old index.html must be untouched"
        );
        assert_eq!(
            read_bytes(
                fixture.path(),
                "lib/apps/fabro-spa/assets/assets/entry-old.js"
            ),
            b"old",
            "old entry asset must be untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn publish_failure_before_flip_leaves_old_bundle_consistent() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = tempfile::tempdir().expect("creating fixture");
        // Staging snapshot of a NEW build, with one unreadable source file
        // that sorts before index.html: the additive copy phase must abort
        // before the index flip.
        write_file(
            fixture.path(),
            "staging/index.html",
            br#"<html><script src="/assets/entry-new.js"></script></html>"#,
        );
        write_file(fixture.path(), "staging/assets/aaa-blocked.js", b"blocked");
        write_file(fixture.path(), "staging/assets/entry-new.js", b"new");
        let blocked = fixture.path().join("staging/assets/aaa-blocked.js");
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000))
            .expect("revoking read permission");

        // Embedded OLD bundle.
        write_file(
            fixture.path(),
            "assets/index.html",
            br#"<html><script src="/assets/entry-old.js"></script></html>"#,
        );
        write_file(fixture.path(), "assets/assets/entry-old.js", b"old");

        let error = publish_staging(
            &fixture.path().join("staging"),
            &fixture.path().join("assets"),
        )
        .expect_err("unreadable source must abort the publish");

        assert!(
            error.to_string().contains("aaa-blocked.js"),
            "error should name the unreadable file: {error:#}"
        );
        assert_eq!(
            read_bytes(fixture.path(), "assets/index.html"),
            br#"<html><script src="/assets/entry-old.js"></script></html>"#,
            "index.html must not have flipped"
        );
        assert!(
            fixture.path().join("assets/assets/entry-old.js").is_file(),
            "old entry asset must survive the aborted publish"
        );
    }

    #[test]
    fn publish_staging_publishes_new_bundle_and_cleans_stale() {
        let fixture = tempfile::tempdir().expect("creating fixture");
        write_file(
            fixture.path(),
            "staging/index.html",
            br#"<html><script src="/assets/entry-new.js"></script></html>"#,
        );
        write_file(fixture.path(), "staging/assets/entry-new.js", b"new");
        write_file(fixture.path(), "assets/index.html", b"old");
        write_file(fixture.path(), "assets/assets/entry-old.js", b"old");

        publish_staging(
            &fixture.path().join("staging"),
            &fixture.path().join("assets"),
        )
        .expect("publishing staged bundle");

        assert_eq!(
            read_bytes(fixture.path(), "assets/index.html"),
            br#"<html><script src="/assets/entry-new.js"></script></html>"#
        );
        assert!(fixture.path().join("assets/assets/entry-new.js").is_file());
        assert!(!fixture.path().join("assets/assets/entry-old.js").exists());
    }

    #[test]
    fn publish_staging_without_index_html_refuses() {
        let fixture = tempfile::tempdir().expect("creating fixture");
        write_file(fixture.path(), "staging/assets/app.js", b"app");
        write_file(fixture.path(), "assets/index.html", b"old");

        let error = publish_staging(
            &fixture.path().join("staging"),
            &fixture.path().join("assets"),
        )
        .expect_err("missing index.html should refuse");

        assert!(
            error.to_string().contains("no index.html"),
            "missing index.html should be named: {error:#}"
        );
        assert_eq!(
            read_bytes(fixture.path(), "assets/index.html"),
            b"old",
            "embedded dir must be untouched"
        );
    }

    #[test]
    fn root_relative_references_extracted_deterministically() {
        let html = concat!(
            r#"<link rel="icon" href="/images/favicon.svg">"#,
            "\n",
            r#"<link rel="stylesheet" href="/assets/app-abc.css?v=3">"#,
            "\n",
            r#"<script src="/assets/entry-xyz.js#hash"></script>"#,
            r#"<img src="data:image/png;base64,AAAA">"#,
            r#"<a href="https://example.com/x">"#,
            r#"<a href="//cdn.example.com/y">"#,
            r#"<link href="/assets/app-abc.css?v=9">"#,
        );
        let references = extract_root_relative_references(html);
        assert_eq!(references, vec![
            "assets/app-abc.css".to_string(),
            "assets/entry-xyz.js".to_string(),
            "images/favicon.svg".to_string(),
        ]);
    }

    #[test]
    fn spa_lock_rejects_second_holder_and_releases_on_drop() {
        let fixture = tempfile::tempdir().expect("creating fixture");
        let root = fixture.path();
        std::fs::create_dir_all(root.join("tmp")).expect("creating tmp");

        let first = SpaRefreshLock::acquire(root).expect("first holder acquires");
        let error =
            SpaRefreshLock::acquire(root).expect_err("second holder must be rejected while held");
        assert!(
            error
                .to_string()
                .contains("another spa refresh/check is running"),
            "lock error should explain the conflict: {error:#}"
        );
        drop(first);

        let second = SpaRefreshLock::acquire(root).expect("lock reusable after release");
        drop(second);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn spa_lock_breaks_stale_dead_pid() {
        let fixture = tempfile::tempdir().expect("creating fixture");
        let root = fixture.path();
        std::fs::create_dir_all(root.join("tmp")).expect("creating tmp");
        // A pid that is valid syntactically but not running (and below the
        // default pid_max) simulates a crashed holder.
        let stale_pid = 4_000_000u32;
        assert!(
            !std::path::Path::new(&format!("/proc/{stale_pid}")).exists(),
            "test assumes pid {stale_pid} is not running"
        );
        std::fs::write(
            root.join("tmp/fabro-dev-spa.lock"),
            format!("{stale_pid}\n"),
        )
        .expect("writing stale lock");

        let lock = SpaRefreshLock::acquire(root).expect("stale lock must be broken");
        drop(lock);
    }
}
