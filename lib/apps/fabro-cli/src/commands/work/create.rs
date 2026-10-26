use anyhow::{Result, bail};

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
    // Structured vocabulary: --type/--priority DERIVE the standard labels,
    // so a caller never hand-spells them (the review-item filing incident:
    // unprefixed `bug`/`task` labels needed a gh cleanup pass). An explicit
    // --label carrying a DIFFERENT value of the same family is a hard
    // error — silent disagreement would file two labels of one family.
    if let Some(item_type) = args.item_type {
        let label = format!("type:{item_type}");
        reject_conflicting_family(&args.labels, "type", &label)?;
        labels.push(label);
    }
    if let Some(priority) = args.priority {
        let label = format!("prio:p{priority}");
        reject_conflicting_family(&args.labels, "prio", &label)?;
        labels.push(label);
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

/// One label family (type:/prio:) may carry exactly one value: a raw
/// `--label` that names a DIFFERENT value of a family a structured flag
/// already set is a hard error, the identical value passes (it dedups
/// server-side).
fn reject_conflicting_family(raw_labels: &[String], family: &str, derived: &str) -> Result<()> {
    let prefix = format!("{family}:");
    for label in raw_labels {
        if label.starts_with(&prefix) && label != derived {
            bail!(
                "conflicting {family} labels: --label {label:?} contradicts {derived:?} \
                 (drop the raw label or align it)"
            );
        }
    }
    Ok(())
}

/// `--body` for short items, `--body-file` (or `-` for stdin) for real
/// bodies: tracker text must never travel through a shell command line.
fn read_body(args: &WorkCreateArgs) -> Result<String> {
    super::read_optional_body(args.body.as_deref(), args.body_file.as_deref())?.ok_or_else(|| {
        anyhow::anyhow!(
            "provide the item body with --body <text> or --body-file <path> (`-` reads stdin)"
        )
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::{read_body, reject_conflicting_family};
    use crate::args::WorkCreateArgs;

    fn create_args(body: Option<&str>, body_file: Option<&str>) -> WorkCreateArgs {
        WorkCreateArgs {
            project:   "fabro".to_string(),
            title:     "a title".to_string(),
            body:      body.map(str::to_string),
            body_file: body_file.map(Into::into),
            lane:      None,
            item_type: None,
            priority:  None,
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
    fn one_label_family_carries_exactly_one_value() {
        // the identical raw label passes (dedup is the server's)
        assert!(reject_conflicting_family(&["type:bug".to_string()], "type", "type:bug").is_ok());
        assert!(reject_conflicting_family(&[], "prio", "prio:p1").is_ok());
        // a DIFFERENT value of the same family is a hard error naming both
        let error = reject_conflicting_family(&["type:task".to_string()], "type", "type:bug")
            .expect_err("a contradicting family label is refused");
        let message = error.to_string();
        assert!(message.contains("type:task"), "{message}");
        assert!(message.contains("type:bug"), "{message}");
    }

    #[test]
    fn a_missing_body_is_refused_with_both_inputs_named() {
        let error = read_body(&create_args(None, None)).expect_err("no body input is refused");
        let message = error.to_string();
        assert!(message.contains("--body"), "{message}");
        assert!(message.contains("--body-file"), "{message}");
    }
}
