//! Command-layer binding for `fabro seeds` (fabro-088b, fork A): the
//! seeds crate's lifted `commands` API is the single source of sd-parity
//! semantics. This module maps fabro clap args onto the typed inputs
//! 1:1, prints the `CommandOutcome` bytes verbatim, and maps `success`
//! onto the process exit code.

use seeds::commands::{
    self, CloseInput, CommandContext as SeedsContext, CommandOutcome, CreateInput, DepAddInput,
    DepListInput, DepRemoveInput, PrimeInput, QueryCommand, QueryInput, ShowInput, UpdateInput,
};

use crate::args::{SeedsCommand, SeedsCreateArgs, SeedsDepCommand, SeedsNamespace, SeedsQueryArgs};
use crate::command_context::CommandContext;

pub(crate) fn dispatch(ns: SeedsNamespace, _base_ctx: &CommandContext) {
    let outcome = match ns.command {
        SeedsCommand::Create(args) => create(&args),
        SeedsCommand::Show(args) => {
            let ctx = SeedsContext::from_cwd();
            commands::show(&ctx, &ShowInput {
                ids:       args.ids,
                format:    args.format,
                json:      args.json,
                json_flag: args.json,
            })
        }
        SeedsCommand::List(args) => {
            let ctx = SeedsContext::from_cwd();
            commands::list(&ctx, &query_input(QueryCommand::List, None, args))
        }
        SeedsCommand::Ready(args) => {
            let ctx = SeedsContext::from_cwd();
            commands::ready(&ctx, &query_input(QueryCommand::Ready, None, args))
        }
        SeedsCommand::Search(args) => {
            let ctx = SeedsContext::from_cwd();
            commands::search(
                &ctx,
                &query_input(QueryCommand::Search, Some(args.needle), args.filters),
            )
        }
        SeedsCommand::Update(args) => {
            let ctx = SeedsContext::from_cwd();
            commands::update(&ctx, &UpdateInput {
                id:               args.id,
                status:           args.status,
                title:            args.title,
                assignee:         args.assignee,
                description:      args.description,
                kind:             args.kind,
                priority:         args.priority,
                add_label:        args.add_label,
                remove_label:     args.remove_label,
                set_labels:       args.set_labels,
                extensions:       args.extensions,
                clear_extensions: args.clear_extensions,
                json:             args.json,
            })
        }
        SeedsCommand::Close(args) => {
            let ctx = SeedsContext::from_cwd();
            commands::close(&ctx, &CloseInput {
                ids:    args.ids,
                reason: args.reason,
                json:   args.json,
            })
        }
        SeedsCommand::Dep(ns) => dep(ns.command),
        SeedsCommand::Prime(args) => commands::prime(&PrimeInput {
            compact: args.compact,
            json:    args.json,
        }),
    };
    emit(&outcome);
}

fn create(args: &SeedsCreateArgs) -> CommandOutcome {
    let ctx = SeedsContext::from_cwd();
    commands::create(&ctx, &CreateInput {
        title:       args.title.clone(),
        kind:        args.kind.clone(),
        priority:    args.priority.clone(),
        description: args.description.clone(),
        labels:      args.labels.clone(),
        assignee:    args.assignee.clone(),
        json:        args.json,
    })
}

fn dep(ns: SeedsDepCommand) -> CommandOutcome {
    let ctx = SeedsContext::from_cwd();
    match ns {
        SeedsDepCommand::Add(args) => commands::dep_add(&ctx, &DepAddInput {
            ids:  args.ids,
            json: args.json,
        }),
        SeedsDepCommand::Remove(args) => commands::dep_remove(&ctx, &DepRemoveInput {
            ids:  args.ids,
            json: args.json,
        }),
        SeedsDepCommand::List(args) => commands::dep_list(&ctx, &DepListInput {
            id:   args.id,
            json: args.json,
        }),
    }
}

/// Mirrors the shared query flags onto the library input; values stay
/// raw strings — the library owns parsing and error envelopes.
fn query_input(command: QueryCommand, query: Option<String>, args: SeedsQueryArgs) -> QueryInput {
    QueryInput {
        command: Some(command),
        query,
        status: args.status,
        kind: args.kind,
        assignee: args.assignee,
        all: args.all,
        label: args.label,
        label_any: args.label_any,
        unlabeled: args.unlabeled,
        priority: args.priority,
        priority_max: args.priority_max,
        limit: args.limit,
        sort: args.sort,
        format: args.format,
        json: args.json,
        respect_schedule: args.respect_schedule,
    }
}

/// Prints the outcome's bytes verbatim and maps `success` onto the exit
/// code — the same contract the reference binary has.
#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "re-printing the library's prepared CLI bytes is this command's purpose"
)]
fn emit(outcome: &CommandOutcome) {
    print!("{}", outcome.stdout);
    if !outcome.stderr.is_empty() {
        eprint!("{}", outcome.stderr);
    }
    if !outcome.success {
        std::process::exit(1);
    }
}
