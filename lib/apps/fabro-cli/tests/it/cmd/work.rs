use fabro_test::{fabro_json_snapshot, fabro_snapshot, test_context};
use httpmock::MockServer;
use serde_json::json;

fn item_json(item_key: &str, title: &str, state: &str) -> serde_json::Value {
    item_json_with_body(
        item_key,
        title,
        state,
        &format!("body of {item_key}\nsecond line"),
    )
}

fn item_json_with_body(item_key: &str, title: &str, state: &str, body: &str) -> serde_json::Value {
    json!({
        "itemKey": item_key,
        "title": title,
        "body": body,
        "state": state,
        "labels": ["project:fabro", "lane:loop"],
        "priority": 2,
        "url": format!("https://github.com/{item_key}"),
        "updatedAt": "2026-10-25T18:04:05Z"
    })
}

/// The close response shape (ADR-0038): the item plus the radius items a
/// forced close overrode — empty for an unforced close.
fn close_response_json(item: &serde_json::Value) -> serde_json::Value {
    json!({
        "item": item,
        "overriddenRadius": []
    })
}

fn project_json(lanes: &[(&str, &str)]) -> serde_json::Value {
    json!({
        "projectId": "fabro",
        "displayName": "Fabro",
        "trackerProvider": "github",
        "trackerRepo": "denkhaus/work",
        "targetRepo": "denkhaus/fabro",
        "projectLabel": "project:fabro",
        "laneLabels": lanes
            .iter()
            .map(|(lane, label)| ((*lane).to_string(), (*label).to_string()))
            .collect::<std::collections::HashMap<_, _>>(),
        "claimLabelPrefix": "fabro:run:",
        "claimCommentEnabled": false,
        "backpressureOpenPrCap": 0
    })
}

#[test]
fn help() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args(["work", "--help"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    Read and write work items and read their claims (ADR-0032)

    Usage: fabro work [OPTIONS] <COMMAND>

    Commands:
      list    List a project's open work items
      show    Show one work item, body included
      create  File a new work item
      close   Close a work item with a reason
      update  Edit a work item's title and/or body
      lane    Assign a work item to a lane, or clear its lane (`--lane none`)
      link    Link a work item to an observation record (ADR-0038 backfill)
      unlink  Remove a work-item observation link
      claims  List a project's live claims
      help    Print this message or the help of the given subcommand(s)

    Options:
          --json              Output as JSON [env: FABRO_JSON=]
          --server <SERVER>   Fabro server target: http(s) URL or absolute Unix socket path [env: FABRO_SERVER=]
          --debug             Enable DEBUG-level logging (default is INFO) [env: FABRO_DEBUG=]
          --no-upgrade-check  Disable automatic upgrade check [env: FABRO_NO_UPGRADE_CHECK=true]
          --quiet             Suppress non-essential output [env: FABRO_QUIET=]
          --verbose           Enable verbose output [env: FABRO_VERBOSE=]
      -h, --help              Print help
    ----- stderr -----
    ");
}

#[test]
fn create_help() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args(["work", "create", "--help"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    File a new work item

    Usage: fabro work create [OPTIONS] --project <PROJECT> --title <TITLE>

    Options:
          --json                 Output as JSON [env: FABRO_JSON=]
          --project <PROJECT>    Work project id
          --debug                Enable DEBUG-level logging (default is INFO) [env: FABRO_DEBUG=]
          --title <TITLE>        Item title
          --body <BODY>          Item body
          --no-upgrade-check     Disable automatic upgrade check [env: FABRO_NO_UPGRADE_CHECK=true]
          --body-file <PATH>     Read the item body from a file (`-` reads stdin)
          --quiet                Suppress non-essential output [env: FABRO_QUIET=]
          --lane <LANE>          Lane to file the item in (translated through the project's lane labels)
          --verbose              Enable verbose output [env: FABRO_VERBOSE=]
          --type <ITEM_TYPE>     Item type (files the `type:<t>` label) [possible values: task, bug, feature, epic]
          --priority <PRIORITY>  Item priority 0-4 (files the `prio:pN` label; default stays the tracker's p2)
          --label <LABELS>       Raw tracker label; repeatable or comma-separated
      -h, --help                 Print help
    ----- stderr -----
    ");
}

#[test]
fn show_help() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args(["work", "show", "--help"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    Show one work item, body included

    Usage: fabro work show [OPTIONS] --project <PROJECT> <SELECTOR>

    Arguments:
      <SELECTOR>  Item key (`owner/repo#12`), bare number, or legacy id (`fabro-d5f0`)

    Options:
          --json               Output as JSON [env: FABRO_JSON=]
          --project <PROJECT>  Work project id
          --debug              Enable DEBUG-level logging (default is INFO) [env: FABRO_DEBUG=]
          --no-upgrade-check   Disable automatic upgrade check [env: FABRO_NO_UPGRADE_CHECK=true]
          --quiet              Suppress non-essential output [env: FABRO_QUIET=]
          --verbose            Enable verbose output [env: FABRO_VERBOSE=]
      -h, --help               Print help
    ----- stderr -----
    ");
}

#[test]
fn list_renders_open_items_and_forwards_the_filters() {
    let context = test_context!();
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/projects/fabro/work-items")
            .query_param("lane", "loop")
            .query_param("limit", "10");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                json!({ "data": [item_json("denkhaus/work#12", "a lane item", "open")] })
                    .to_string(),
            );
    });
    let project = server.mock(|when, then| {
        when.method("GET").path("/api/v1/projects/fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(project_json(&[("loop", "lane:loop")]).to_string());
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args([
        "work",
        "list",
        "--project",
        "fabro",
        "--lane",
        "loop",
        "--limit",
        "10",
    ]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    KEY               PRIO  STATE  LABELS                   TITLE       
     denkhaus/work#12  2     open   project:fabro,lane:loop  a lane item
    ----- stderr -----
    ");

    // The lane name is checked against the registry, and the filters travel
    // as query parameters.
    project.assert();
    mock.assert();
}

#[test]
fn list_refuses_an_unknown_lane_before_listing() {
    let context = test_context!();
    let server = MockServer::start();
    let project = server.mock(|when, then| {
        when.method("GET").path("/api/v1/projects/fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(project_json(&[("develop", "lane:develop"), ("loop", "lane:loop")]).to_string());
    });
    let items = server.mock(|when, then| {
        when.method("GET").path("/api/v1/projects/fabro/work-items");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(json!({ "data": [] }).to_string());
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args(["work", "list", "--project", "fabro", "--lane", "revisor"]);
    fabro_snapshot!(context.filters(), cmd, @r#"
    success: false
    exit_code: 1
    ----- stdout -----
    ----- stderr -----
      × work project fabro declares no lane "revisor" (declared lanes: develop, loop)
    "#);

    project.assert();
    items.assert_calls(0);
}

#[test]
fn list_refuses_a_limit_outside_the_api_range() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args(["work", "list", "--project", "fabro", "--limit", "500"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: false
    exit_code: 2
    ----- stdout -----
    ----- stderr -----
    error: invalid value '500' for '--limit <LIMIT>': 500 is not in 1..=100

    For more information, try '--help'.
    ");
}

#[test]
fn list_json_returns_the_items() {
    let context = test_context!();
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method("GET").path("/api/v1/projects/fabro/work-items");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                // A single-line body on purpose: the JSON snapshot filters
                // normalize backslashes, which would turn a `\n` in the body
                // into `/n` and make the snapshot read like a bug.
                json!({
                    "data": [item_json_with_body(
                        "denkhaus/work#12",
                        "a lane item",
                        "open",
                        "a single-line body"
                    )]
                })
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let output = context
        .command()
        .args(["--json", "work", "list", "--project", "fabro"])
        .output()
        .expect("command should run");

    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    fabro_json_snapshot!(context.filters(), value, @r#"
    [
      {
        "body": "a single-line body",
        "itemKey": "denkhaus/work#12",
        "labels": [
          "project:fabro",
          "lane:loop"
        ],
        "priority": 2,
        "state": "open",
        "title": "a lane item",
        "updatedAt": "[TIMESTAMP]",
        "url": "https://github.com/denkhaus/work#12"
      }
    ]
    "#);
    mock.assert();
}

/// A bare number is repository-relative: the CLI reads the project row once
/// and composes `denkhaus/work#12` before the item request.
#[test]
fn show_resolves_a_bare_number_against_the_project_tracker() {
    let context = test_context!();
    let server = MockServer::start();
    let project = server.mock(|when, then| {
        when.method("GET").path("/api/v1/projects/fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(project_json(&[("loop", "lane:loop")]).to_string());
    });
    let item = server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/projects/fabro/work-item")
            .query_param("itemKey", "denkhaus/work#12");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(item_json("denkhaus/work#12", "a lane item", "open").to_string());
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args(["work", "show", "--project", "fabro", "12"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    FIELD     VALUE                               
     key       denkhaus/work#12                    
     state     open                                
     priority  2                                   
     labels    project:fabro,lane:loop             
     title     a lane item                         
     url       https://github.com/denkhaus/work#12 
     updated   2026-10-25T18:04:05+00:00

    body of denkhaus/work#12
    second line
    ----- stderr -----
    ");

    project.assert();
    item.assert();
}

/// A legacy id goes to the server unchanged: the migration-label seam
/// (stage 1a) owns the resolution, so the CLI must not read the project row
/// first.
#[test]
fn show_passes_a_legacy_id_through_without_a_project_lookup() {
    let context = test_context!();
    let server = MockServer::start();
    let project = server.mock(|when, then| {
        when.method("GET").path("/api/v1/projects/fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(project_json(&[("loop", "lane:loop")]).to_string());
    });
    let item = server.mock(|when, then| {
        when.method("GET")
            .path("/api/v1/projects/fabro/work-item")
            .query_param("itemKey", "fabro-d5f0");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(item_json("denkhaus/work#6", "a migrated item", "closed").to_string());
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args(["work", "show", "--project", "fabro", "fabro-d5f0"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    FIELD     VALUE                              
     key       denkhaus/work#6                    
     state     closed                             
     priority  2                                  
     labels    project:fabro,lane:loop            
     title     a migrated item                    
     url       https://github.com/denkhaus/work#6 
     updated   2026-10-25T18:04:05+00:00

    body of denkhaus/work#6
    second line
    ----- stderr -----
    ");

    item.assert();
    project.assert_calls(0);
}

#[test]
fn show_refuses_a_selector_outside_the_three_shapes() {
    let context = test_context!();
    let server = MockServer::start();
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args(["work", "show", "--project", "fabro", "denkhaus/work"]);
    fabro_snapshot!(context.filters(), cmd, @r#"
    success: false
    exit_code: 1
    ----- stdout -----
    ----- stderr -----
      × invalid work item selector "denkhaus/work": pass a full key (`owner/repo#12`), a bare number (`12`), or a legacy id (`fabro-d5f0`)
    "#);
}

/// The structured vocabulary derives the standard labels: --type and
/// --priority file type:<t>/prio:pN so a caller never hand-spells them
/// (the review-item filing incident: unprefixed labels needed a cleanup).
#[test]
fn create_derives_type_and_priority_labels() {
    let context = test_context!();
    let server = MockServer::start();
    let create = server.mock(|when, then| {
        when.method("POST")
            .path("/api/v1/projects/fabro/work-items")
            .body_includes("\"labels\":[\"type:bug\",\"prio:p1\",\"residual\"]");
        then.status(201)
            .header("Content-Type", "application/json")
            .body(item_json("denkhaus/work#16", "a review gap", "open").to_string());
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args([
        "work",
        "create",
        "--project",
        "fabro",
        "--title",
        "a review gap",
        "--body",
        "the body",
        "--type",
        "bug",
        "--priority",
        "1",
        "--label",
        "residual",
    ]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    denkhaus/work#16
    ----- stderr -----
    ");

    create.assert();
}

/// A raw --label that CONTRADICTS the structured flag is refused before
/// the server sees anything — one family, one value.
#[test]
fn create_refuses_a_contradicting_family_label() {
    let context = test_context!();
    let server = MockServer::start();
    let create = server.mock(|when, then| {
        when.method("POST")
            .path("/api/v1/projects/fabro/work-items");
        then.status(201);
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args([
        "work",
        "create",
        "--project",
        "fabro",
        "--title",
        "a review gap",
        "--body",
        "the body",
        "--type",
        "bug",
        "--label",
        "type:task",
    ]);
    fabro_snapshot!(context.filters(), cmd, @r#"
    success: false
    exit_code: 1
    ----- stdout -----
    ----- stderr -----
      × conflicting type labels: --label "type:task" contradicts "type:bug" (drop the raw label or align it)
    "#);

    create.assert_calls(0);
}

#[test]
fn create_reads_the_body_from_a_file_and_translates_the_lane() {
    let context = test_context!();
    let server = MockServer::start();
    let project = server.mock(|when, then| {
        when.method("GET").path("/api/v1/projects/fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(project_json(&[("loop", "lane:loop")]).to_string());
    });
    let create = server.mock(|when, then| {
        when.method("POST")
            .path("/api/v1/projects/fabro/work-items")
            .body_includes("\"title\":\"a filed item\"")
            .body_includes("line one\\nline two")
            .body_includes("lane:loop")
            .body_includes("residual");
        then.status(201)
            .header("Content-Type", "application/json")
            .body(item_json("denkhaus/work#13", "a filed item", "open").to_string());
    });
    context.set_http_target(&server.base_url());

    let body_path = context
        .write_home("item-body.md", "line one\nline two\n")
        .home_dir
        .join("item-body.md");

    let mut cmd = context.command();
    cmd.args([
        "work",
        "create",
        "--project",
        "fabro",
        "--title",
        "a filed item",
        "--body-file",
        body_path.to_str().expect("utf-8 path"),
        "--lane",
        "loop",
        "--label",
        "residual",
    ]);
    // Text mode prints the new key alone so a script can capture it.
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    denkhaus/work#13
    ----- stderr -----
    ");

    project.assert();
    create.assert();
}

#[test]
fn create_reads_the_body_from_stdin() {
    let context = test_context!();
    let server = MockServer::start();
    let create = server.mock(|when, then| {
        when.method("POST")
            .path("/api/v1/projects/fabro/work-items")
            .body_includes("piped body");
        then.status(201)
            .header("Content-Type", "application/json")
            .body(item_json("denkhaus/work#14", "piped", "open").to_string());
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args([
        "work",
        "create",
        "--project",
        "fabro",
        "--title",
        "piped",
        "--body-file",
        "-",
    ])
    .write_stdin("piped body\n");
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    denkhaus/work#14
    ----- stderr -----
    ");

    create.assert();
}

#[test]
fn create_refuses_an_unknown_lane_before_writing() {
    let context = test_context!();
    let server = MockServer::start();
    let project = server.mock(|when, then| {
        when.method("GET").path("/api/v1/projects/fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(project_json(&[("develop", "lane:develop"), ("loop", "lane:loop")]).to_string());
    });
    let create = server.mock(|when, then| {
        when.method("POST")
            .path("/api/v1/projects/fabro/work-items");
        then.status(201);
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args([
        "work",
        "create",
        "--project",
        "fabro",
        "--title",
        "a filed item",
        "--body",
        "body",
        "--lane",
        "revisor",
    ]);
    fabro_snapshot!(context.filters(), cmd, @r#"
    success: false
    exit_code: 1
    ----- stdout -----
    ----- stderr -----
      × work project fabro declares no lane "revisor" (declared lanes: develop, loop)
    "#);

    project.assert();
    create.assert_calls(0);
}

#[test]
fn update_help() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args(["work", "update", "--help"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    Edit a work item's title and/or body

    Usage: fabro work update [OPTIONS] --project <PROJECT> <SELECTOR>

    Arguments:
      <SELECTOR>  Item key (`owner/repo#12`), bare number, or legacy id (`fabro-d5f0`)

    Options:
          --json                   Output as JSON [env: FABRO_JSON=]
          --project <PROJECT>      Work project id
          --debug                  Enable DEBUG-level logging (default is INFO) [env: FABRO_DEBUG=]
          --title <TITLE>          New item title (empty keeps the current one)
          --body <BODY>            New item body
          --no-upgrade-check       Disable automatic upgrade check [env: FABRO_NO_UPGRADE_CHECK=true]
          --body-file <BODY_FILE>  Read the new item body from a file (`-` reads stdin)
          --quiet                  Suppress non-essential output [env: FABRO_QUIET=]
          --verbose                Enable verbose output [env: FABRO_VERBOSE=]
      -h, --help                   Print help
    ----- stderr -----
    ");
}

/// The wayfinding append path (ADR-0032): title travels as the flag
/// (short, not tracker prose), the body through stdin — the wire pin
/// proves both reach the update endpoint under the resolved key.
#[test]
fn update_sends_the_title_and_a_stdin_body_under_the_resolved_key() {
    let context = test_context!();
    let server = MockServer::start();
    let update = server.mock(|when, then| {
        when.method("POST")
            .path("/api/v1/projects/fabro/work-items/update")
            .body_includes("\"itemKey\":\"denkhaus/work#12\"")
            .body_includes("\"title\":\"A sharper title\"")
            .body_includes("appended by the resolver");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(item_json("denkhaus/work#12", "A sharper title", "open").to_string());
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args([
        "work",
        "update",
        "--project",
        "fabro",
        "denkhaus/work#12",
        "--title",
        "A sharper title",
        "--body-file",
        "-",
    ])
    .write_stdin("appended by the resolver\n");
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    ----- stderr -----
    Updated denkhaus/work#12.
    ");

    update.assert();
}

/// Neither title nor body is an error BEFORE any server call — the
/// refusal never reaches the endpoint. Whitespace-only input is the
/// empty edit too (the first run of this verb shipped that gap: a
/// blank stdin body sailed through the CLI check and was refused by
/// the server instead).
#[test]
fn update_refuses_an_empty_edit_before_any_server_call() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args(["work", "update", "--project", "fabro", "denkhaus/work#12"]);
    fabro_snapshot!(context.filters(), cmd, @r#"
    success: false
    exit_code: 1
    ----- stdout -----
    ----- stderr -----
      × an update needs a title or a body — pass --title, --body, or --body-file - (stdin)
    "#);

    let mut cmd = context.command();
    cmd.args([
        "work",
        "update",
        "--project",
        "fabro",
        "denkhaus/work#12",
        "--body-file",
        "-",
    ])
    .write_stdin("   \n\n");
    fabro_snapshot!(context.filters(), cmd, @r#"
    success: false
    exit_code: 1
    ----- stdout -----
    ----- stderr -----
      × an update needs a title or a body — pass --title, --body, or --body-file - (stdin)
    "#);
}

#[test]
fn close_sends_the_reason_and_reports_the_key() {
    let context = test_context!();
    let server = MockServer::start();
    let close = server.mock(|when, then| {
        when.method("POST")
            .path("/api/v1/projects/fabro/work-items/close")
            .body_includes("denkhaus/work#12")
            .body_includes("landed in 84caa6f98");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                close_response_json(&item_json("denkhaus/work#12", "a lane item", "closed"))
                    .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args([
        "work",
        "close",
        "--project",
        "fabro",
        "denkhaus/work#12",
        "--reason",
        "landed in 84caa6f98",
    ]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    ----- stderr -----
    Closed denkhaus/work#12 (closed).
    ");

    close.assert();
}

/// The other side of the evidence rule (retro finding 2026-10-26): an
/// empty or whitespace-only reason is refused at the CLI boundary
/// before any server call — the item body must carry its evidence.
#[test]
fn close_refuses_an_empty_reason_before_any_server_call() {
    let context = test_context!();
    let mut cmd = context.command();
    cmd.args([
        "work",
        "close",
        "--project",
        "fabro",
        "denkhaus/work#12",
        "--reason",
        "   ",
    ]);
    fabro_snapshot!(context.filters(), cmd, @r#"
    success: false
    exit_code: 1
    ----- stdout -----
    ----- stderr -----
      × a close reason is required — pass --reason "<evidence>" (run ids, PR, commit)
    "#);
}

#[test]
fn claims_renders_the_live_queue() {
    let context = test_context!();
    let server = MockServer::start();
    let claims = server.mock(|when, then| {
        when.method("GET").path("/api/v1/projects/fabro/claims");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(
                json!({
                    "data": [{
                        "claimId": "9ff5d7c8-1a6c-4f4e-9d2f-8f9e0f0b6a13",
                        "projectId": "fabro",
                        "itemKey": "denkhaus/work#12",
                        "runId": "01M5SATCK7Z4XEGY6V0J4ZJQ03",
                        "lane": "loop",
                        "state": "awaiting_merge",
                        "grantedAt": "2026-10-25T18:00:00Z",
                        "updatedAt": "2026-10-25T18:10:00Z",
                        "prUrl": "https://github.com/denkhaus/fabro/pull/496"
                    }]
                })
                .to_string(),
            );
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args(["work", "claims", "--project", "fabro"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    CLAIM                                 ITEM              RUN                         LANE  STATE           GRANTED                    PR                                         
     9ff5d7c8-1a6c-4f4e-9d2f-8f9e0f0b6a13  denkhaus/work#12  [ULID]  loop  awaiting_merge  2026-10-25T18:00:00+00:00  https://github.com/denkhaus/fabro/pull/496
    ----- stderr -----
    ");

    claims.assert();
}

#[test]
fn claims_reports_an_empty_ledger_without_failing() {
    let context = test_context!();
    let server = MockServer::start();
    let claims = server.mock(|when, then| {
        when.method("GET").path("/api/v1/projects/fabro/claims");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(json!({ "data": [] }).to_string());
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args(["work", "claims", "--project", "fabro"]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    ----- stderr -----
    No live claims in project fabro.
    ");

    claims.assert();
}

/// The tranche move (work#1065): a lane NAME travels to the server
/// after the CLI validated it against the project's declared lanes —
/// the same registry posture as `--lane` on create.
#[test]
fn lane_assigns_after_validating_the_lane_name() {
    let context = test_context!();
    let server = MockServer::start();
    let project = server.mock(|when, then| {
        when.method("GET").path("/api/v1/projects/fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(project_json(&[("loop", "lane:loop"), ("develop", "lane:develop")]).to_string());
    });
    let lane = server.mock(|when, then| {
        when.method("POST")
            .path("/api/v1/projects/fabro/work-items/lane")
            .body_includes("\"itemKey\":\"denkhaus/work#12\"")
            .body_includes("\"lane\":\"loop\"");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(item_json("denkhaus/work#12", "a lane item", "open").to_string());
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args([
        "work",
        "lane",
        "--project",
        "fabro",
        "denkhaus/work#12",
        "--lane",
        "loop",
    ]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    ----- stderr -----
    Moved denkhaus/work#12 to loop.
    ");

    project.assert();
    lane.assert();
}

/// `--lane none` clears the lane without the registry lookup — the
/// request carries no lane at all.
#[test]
fn lane_none_clears_the_lane_without_a_registry_lookup() {
    let context = test_context!();
    let server = MockServer::start();
    let project = server.mock(|when, then| {
        when.method("GET").path("/api/v1/projects/fabro");
        then.status(200);
    });
    let lane = server.mock(|when, then| {
        when.method("POST")
            .path("/api/v1/projects/fabro/work-items/lane")
            // The generated client omits a `None` lane from the payload —
            // the unassign travels as the field's absence.
            .body_includes("\"itemKey\":\"denkhaus/work#12\"")
            .body_excludes("\"lane\"");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(item_json("denkhaus/work#12", "a lane item", "open").to_string());
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args([
        "work",
        "lane",
        "--project",
        "fabro",
        "denkhaus/work#12",
        "--lane",
        "none",
    ]);
    fabro_snapshot!(context.filters(), cmd, @"
    success: true
    exit_code: 0
    ----- stdout -----
    ----- stderr -----
    Moved denkhaus/work#12 to none.
    ");

    project.assert_calls(0);
    lane.assert();
}

/// An unknown lane is refused before the server sees the move request.
#[test]
fn lane_refuses_an_unknown_lane_before_the_move() {
    let context = test_context!();
    let server = MockServer::start();
    let project = server.mock(|when, then| {
        when.method("GET").path("/api/v1/projects/fabro");
        then.status(200)
            .header("Content-Type", "application/json")
            .body(project_json(&[("loop", "lane:loop")]).to_string());
    });
    let lane = server.mock(|when, then| {
        when.method("POST")
            .path("/api/v1/projects/fabro/work-items/lane");
        then.status(200);
    });
    context.set_http_target(&server.base_url());

    let mut cmd = context.command();
    cmd.args([
        "work",
        "lane",
        "--project",
        "fabro",
        "denkhaus/work#12",
        "--lane",
        "revisor",
    ]);
    fabro_snapshot!(context.filters(), cmd, @r#"
    success: false
    exit_code: 1
    ----- stdout -----
    ----- stderr -----
      × work project fabro declares no lane "revisor" (declared lanes: loop)
    "#);

    project.assert();
    lane.assert_calls(0);
}
