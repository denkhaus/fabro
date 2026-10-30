#![expect(
    clippy::disallowed_types,
    reason = "sync CLI work commands read the item body through std::io::Read"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "sync CLI work commands read the item body from a plain file or std::io::stdin"
)]

mod claims;
mod close;
mod create;
mod lane;
mod list;
mod show;
mod update;

use std::io::Read as _;

use anyhow::{Context as _, Result, bail};
use cli_table::format::{Border, Separator};
use cli_table::{CellStruct, Table};
use fabro_api::types;
use fabro_client::Client;
use fabro_util::printer::Printer;

use crate::args::{WorkCommand, WorkNamespace};
use crate::command_context::CommandContext;

pub(crate) async fn dispatch(ns: WorkNamespace, base_ctx: &CommandContext) -> Result<()> {
    let ctx = base_ctx.with_target(&ns.target)?;
    match ns.command {
        WorkCommand::List(args) => list::list_command(&args, &ctx).await,
        WorkCommand::Show(args) => show::show_command(&args, &ctx).await,
        WorkCommand::Create(args) => create::create_command(&args, &ctx).await,
        WorkCommand::Close(args) => close::close_command(&args, &ctx).await,
        WorkCommand::Update(args) => update::update_command(&args, &ctx).await,
        WorkCommand::Lane(args) => lane::lane_command(&args, &ctx).await,
        WorkCommand::Claims(args) => claims::claims_command(&args, &ctx).await,
    }
}

/// Renders one titled table with the crate's border and separator defaults:
/// the three read verbs share a shape, so they share one renderer.
fn print_table(
    printer: Printer,
    use_color: bool,
    title: Vec<CellStruct>,
    rows: Vec<Vec<CellStruct>>,
) -> Result<()> {
    let color_choice = if use_color {
        cli_table::ColorChoice::Auto
    } else {
        cli_table::ColorChoice::Never
    };
    let table = rows
        .table()
        .title(title)
        .color_choice(color_choice)
        .border(Border::builder().build())
        .separator(Separator::builder().build());
    fabro_util::printout!(printer, "{}", table.display()?);
    Ok(())
}

/// How one `show`/`close` selector names an item.
///
/// Two shapes are absolute and travel to the server unchanged: a full key
/// (`owner/repo#12`) and a legacy id (`fabro-d5f0`, optionally prefixed
/// `seeds:`), which the server resolves through the migration label the
/// tracker carries. A bare number is relative to the project's tracker
/// repository, so only that shape needs the project row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ItemSelector<'a> {
    Absolute(&'a str),
    BareNumber(&'a str),
}

/// Parses a selector at the CLI boundary. Refusing here, with the three
/// accepted shapes named, keeps a typo from reading as a tracker failure.
fn parse_selector(selector: &str) -> Result<ItemSelector<'_>> {
    let trimmed = selector.trim();
    if !trimmed.is_empty() && trimmed.chars().all(|digit| digit.is_ascii_digit()) {
        return Ok(ItemSelector::BareNumber(trimmed));
    }
    if is_full_key(trimmed) || is_legacy_id(trimmed) {
        return Ok(ItemSelector::Absolute(trimmed));
    }
    bail!(
        "invalid work item selector {selector:?}: pass a full key (`owner/repo#12`), a bare \
         number (`12`), or a legacy id (`fabro-d5f0`)"
    )
}

/// The provider-stable key the server expects, resolving a bare number
/// against the project's tracker repository.
async fn resolve_item_key(client: &Client, project_id: &str, selector: &str) -> Result<String> {
    match parse_selector(selector)? {
        ItemSelector::Absolute(key) => Ok(key.to_string()),
        ItemSelector::BareNumber(number) => {
            let project = project_row(client, project_id).await?;
            Ok(format!("{}#{number}", project.tracker_repo))
        }
    }
}

/// The registered project row: the tracker repository a bare number composes
/// against, and the lane labels that name a queue.
async fn project_row(client: &Client, project_id: &str) -> Result<types::WorkProject> {
    client
        .retrieve_work_project(project_id)
        .await
        .with_context(|| format!("reading work project {project_id}"))
}

/// The shared body-input refusal for create AND update (one message,
/// code-review sprint 55+56 finding — the literal lived twice).
const BODY_INPUT_REFUSAL: &str =
    "provide the item body with --body <text> or --body-file <path> (`-` reads stdin)";

/// The `--body` / `--body-file` input pair shared by create and update:
/// tracker text travels through a flag, a file, or stdin (`-`) — never
/// a shell command line. `None` when the caller provided neither (the
/// verbs decide whether that is an error). clap's `conflicts_with`
/// refuses both flags together, so that arm is defensive only.
pub(super) fn read_optional_body(
    body: Option<&str>,
    body_file: Option<&std::path::Path>,
) -> Result<Option<String>> {
    match (body, body_file) {
        (Some(body), None) => Ok(Some(body.to_string())),
        (None, Some(path)) => read_body_file(path).map(Some),
        (None, None) => Ok(None),
        (Some(_), Some(_)) => bail!(BODY_INPUT_REFUSAL),
    }
}

fn read_body_file(path: &std::path::Path) -> Result<String> {
    if path == std::path::Path::new("-") {
        let mut body = String::new();
        std::io::stdin()
            .read_to_string(&mut body)
            .context("reading the item body from stdin")?;
        return Ok(body);
    }
    std::fs::read_to_string(path)
        .with_context(|| format!("reading the item body from {}", path.display()))
}

/// Translates a lane name into the tracker label the project maps it to.
///
/// An unknown lane is refused, naming the declared lanes: the server's own
/// fallback (`lane:<name>`) would turn a typo into a silently empty queue.
fn lane_label(project: &types::WorkProject, lane: &str) -> Result<String> {
    project.lane_labels.get(lane).cloned().ok_or_else(|| {
        let declared = if project.lane_labels.is_empty() {
            "none".to_string()
        } else {
            let mut lanes: Vec<&str> = project.lane_labels.keys().map(String::as_str).collect();
            lanes.sort_unstable();
            lanes.join(", ")
        };
        anyhow::anyhow!(
            "work project {} declares no lane {lane:?} (declared lanes: {declared})",
            project.project_id
        )
    })
}

/// The same lookup for a caller that only needs the lane to be real: `list`
/// sends the lane NAME (the server translates it), so it never uses the
/// label itself.
fn ensure_known_lane(project: &types::WorkProject, lane: &str) -> Result<()> {
    lane_label(project, lane).map(drop)
}

/// A full key is `<owner>/<repo>#<number>`: both repository segments
/// non-empty and the trailing number numeric, so a truncated selector like
/// `#12` or `/work#1` is refused here instead of at the tracker.
fn is_full_key(selector: &str) -> bool {
    let Some((repo, number)) = selector.split_once('#') else {
        return false;
    };
    let mut segments = repo.split('/');
    let owner = segments.next();
    let name = segments.next();
    let two_segments = segments.next().is_none();
    two_segments
        && owner.is_some_and(|owner| !owner.is_empty())
        && name.is_some_and(|name| !name.is_empty())
        && !number.is_empty()
        && number.chars().all(|digit| digit.is_ascii_digit())
}

/// A legacy id is `fabro-<id>`, optionally written `seeds:fabro-<id>`. The
/// id itself is only checked for being a plausible token, so a typo like
/// `fabro-junk!` is refused here instead of travelling to the tracker.
fn is_legacy_id(selector: &str) -> bool {
    let Some(id) = selector
        .strip_prefix("seeds:")
        .unwrap_or(selector)
        .strip_prefix("fabro-")
    else {
        return false;
    };
    !id.is_empty()
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

/// Comma-joined labels for display; `-` when the item carries none.
fn labels_cell(labels: &[String]) -> String {
    if labels.is_empty() {
        "-".to_string()
    } else {
        labels.join(",")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use fabro_api::types::WorkProject;

    use super::{ItemSelector, is_legacy_id, lane_label, parse_selector};

    fn project(lanes: &[(&str, &str)]) -> WorkProject {
        WorkProject {
            backpressure_open_pr_cap: 0,
            claim_comment_enabled:    false,
            claim_label_prefix:       "fabro:run:".to_string(),
            display_name:             "Fabro".to_string(),
            lane_labels:              lanes
                .iter()
                .map(|(lane, label)| ((*lane).to_string(), (*label).to_string()))
                .collect::<HashMap<_, _>>(),
            project_id:               "fabro".to_string(),
            project_label:            "project:fabro".to_string(),
            target_repo:              None,
            tracker_provider:         "github".to_string(),
            tracker_repo:             "denkhaus/work".to_string(),
        }
    }

    #[test]
    fn a_lane_resolves_through_the_projects_label_map() -> anyhow::Result<()> {
        let project = project(&[("loop", "lane:loop"), ("develop", "lane:develop")]);
        assert_eq!(lane_label(&project, "loop")?, "lane:loop");
        Ok(())
    }

    #[test]
    fn an_unknown_lane_names_the_declared_lanes() {
        let project = project(&[("develop", "lane:develop"), ("loop", "lane:loop")]);
        let error = lane_label(&project, "revisor").expect_err("an unknown lane is refused");
        let message = error.to_string();
        assert!(message.contains("revisor"), "{message}");
        assert!(message.contains("develop, loop"), "{message}");
    }

    #[test]
    fn a_bare_number_is_relative_to_the_project_tracker() {
        assert_eq!(
            parse_selector("12").expect("a bare number is a selector"),
            ItemSelector::BareNumber("12")
        );
        assert_eq!(
            parse_selector(" 0007 ").expect("a padded number is a selector"),
            ItemSelector::BareNumber("0007")
        );
    }

    #[test]
    fn a_full_key_travels_unchanged() {
        assert_eq!(
            parse_selector("denkhaus/work#12").expect("a full key is a selector"),
            ItemSelector::Absolute("denkhaus/work#12")
        );
        assert_eq!(
            parse_selector("other/repo#1").expect("a full key is a selector"),
            ItemSelector::Absolute("other/repo#1")
        );
    }

    #[test]
    fn a_legacy_id_travels_unchanged_for_the_migration_label_seam() {
        for selector in ["fabro-d5f0", "seeds:fabro-d5f0"] {
            assert_eq!(
                parse_selector(selector).expect("a legacy id is a selector"),
                ItemSelector::Absolute(selector)
            );
        }
        assert!(is_legacy_id("fabro-d5f0"));
        assert!(is_legacy_id("seeds:fabro-d5f0"));
        assert!(!is_legacy_id("denkhaus/work#12"));
    }

    #[test]
    fn a_selector_outside_the_three_shapes_is_refused_with_the_shapes_named() {
        let error = parse_selector("work#").expect_err("a key without a number is refused");
        let message = error.to_string();
        for expected in ["owner/repo#12", "bare number", "fabro-d5f0"] {
            assert!(
                message.contains(expected),
                "the refusal must name {expected}: {message}"
            );
        }
        for refused in [
            "",
            "  ",
            "#12",
            "/work#1",
            "denkhaus/work",
            "52-x",
            "fabro-",
            "fabro-junk!",
            "seeds:",
            "seeds:-x",
        ] {
            assert!(
                parse_selector(refused).is_err(),
                "{refused:?} must not parse as a selector"
            );
        }
    }
}
