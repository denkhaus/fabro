#![expect(
    clippy::disallowed_types,
    reason = "sync CLI `work create` command and its body-input tests read through std::io::Read"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "sync CLI `work create` command: the item body comes from a plain file or std::io::stdin"
)]

use std::io::Read as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

use super::{lane_label, project_row};
use crate::args::WorkCreateArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;

/// Files one work item. Text mode prints the new key alone so a script can
/// capture it (`key=$(fabro work create …)`); nothing else goes to stdout.
pub(super) async fn create_command(args: &WorkCreateArgs, ctx: &CommandContext) -> Result<()> {
    let title = args.title.trim();
    if title.is_empty() {
        bail!("the item title must not be empty");
    }
    let body = read_body(args)?;
    if body.trim().is_empty() {
        bail!("the item body must not be empty");
    }

    let client = ctx.server().await?;
    let mut labels: Vec<String> = Vec::new();
    if let Some(lane) = args.lane.as_deref() {
        let project = project_row(&client, &args.project).await?;
        labels.push(lane_label(&project, lane)?);
    }
    labels.extend(args.labels.iter().cloned());

    let item = client
        .create_work_item(&args.project, title, &body, labels)
        .await?;

    let printer = ctx.printer();
    if ctx.json_output() {
        print_json_pretty(&item)?;
    } else {
        fabro_util::printout!(printer, "{}", item.item_key);
    }
    Ok(())
}

/// `--body` for short items, `--body-file` (or `-` for stdin) for real
/// bodies: tracker text must never travel through a shell command line.
fn read_body(args: &WorkCreateArgs) -> Result<String> {
    match (&args.body, &args.body_file) {
        (Some(body), None) => Ok(body.clone()),
        (None, Some(path)) => read_body_file(path),
        // clap's `conflicts_with` refuses both flags together, so this arm is
        // the caller who provided neither.
        _ => bail!(
            "provide the item body with --body <text> or --body-file <path> (`-` reads stdin)"
        ),
    }
}

fn read_body_file(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut body = String::new();
        std::io::stdin()
            .read_to_string(&mut body)
            .context("reading the item body from stdin")?;
        return Ok(body);
    }
    std::fs::read_to_string(path)
        .with_context(|| format!("reading the item body from {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::read_body;
    use crate::args::WorkCreateArgs;

    fn create_args(body: Option<&str>, body_file: Option<&str>) -> WorkCreateArgs {
        WorkCreateArgs {
            project:   "fabro".to_string(),
            title:     "a title".to_string(),
            body:      body.map(str::to_string),
            body_file: body_file.map(Into::into),
            lane:      None,
            labels:    Vec::new(),
        }
    }

    #[test]
    fn the_body_comes_from_the_flag_or_the_file() -> anyhow::Result<()> {
        assert_eq!(read_body(&create_args(Some("inline"), None))?, "inline");

        let mut file = tempfile::NamedTempFile::new()?;
        file.write_all(b"from a file\n")?;
        let path = file.path().to_str().expect("utf-8 temp path");
        let body = read_body(&create_args(None, Some(path)))?;
        assert_eq!(body, "from a file\n");
        Ok(())
    }

    #[test]
    fn a_missing_body_is_refused_with_both_inputs_named() {
        let error = read_body(&create_args(None, None)).expect_err("no body input is refused");
        let message = error.to_string();
        assert!(message.contains("--body"), "{message}");
        assert!(message.contains("--body-file"), "{message}");
    }
}
