use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use fabro_client::{AuthEntry, AuthStore, OAuthEntry};
use serde::Serialize;

use crate::args::AuthStatusArgs;
use crate::command_context::CommandContext;
use crate::shared::print_json_pretty;
use crate::user_config;
use crate::user_config::ServerTarget;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum OAuthState {
    Active,
    ExpiredRefreshable,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind")]
enum StatusRow {
    #[serde(rename = "oauth")]
    OAuth {
        server:                   String,
        oauth_state:              OAuthState,
        access_token_expires_at:  DateTime<Utc>,
        refresh_token_expires_at: DateTime<Utc>,
        logged_in_at:             DateTime<Utc>,
        login:                    String,
        name:                     String,
        email:                    String,
        idp_issuer:               String,
        idp_subject:              String,
    },
    #[serde(rename = "dev-token")]
    DevToken {
        server:       String,
        logged_in_at: DateTime<Utc>,
    },
}

#[derive(Serialize)]
struct StatusOutput {
    servers: Vec<StatusRow>,
}

pub(super) fn status_command(args: &AuthStatusArgs, ctx: &CommandContext) -> Result<()> {
    let printer = ctx.printer();
    let store = AuthStore::default();
    let now = Utc::now();
    let rows = if args.server.as_deref().is_some() {
        let target = user_config::resolve_server_target(&args.server, ctx.user_settings())?;
        filter_rows(&store, &target, now)?
    } else {
        all_rows(&store, now)?
    };
    if ctx.explicit_json_requested() {
        print_json_pretty(&StatusOutput { servers: rows })?;
        return Ok(());
    }

    if rows.is_empty() {
        let env_server = crate::process_env_var(fabro_static::EnvVars::FABRO_SERVER);
        fabro_util::printerr!(
            printer,
            "{}",
            empty_status_message(args.server.as_deref(), env_server.as_deref(), &store)
        );
        return Ok(());
    }

    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            fabro_util::printerr!(printer, "");
        }
        match row {
            StatusRow::OAuth {
                server,
                oauth_state,
                access_token_expires_at,
                refresh_token_expires_at,
                login,
                name,
                email,
                ..
            } => {
                fabro_util::printerr!(printer, "{server}");
                fabro_util::printerr!(
                    printer,
                    "  OAuth: {} as {}",
                    human_state(*oauth_state),
                    login
                );
                fabro_util::printerr!(
                    printer,
                    "  Name: {}",
                    if name.is_empty() {
                        "(not set)"
                    } else {
                        name.as_str()
                    }
                );
                fabro_util::printerr!(
                    printer,
                    "  Email: {}",
                    if email.is_empty() {
                        "(not set)"
                    } else {
                        email.as_str()
                    }
                );
                fabro_util::printerr!(
                    printer,
                    "  Access expires: {}",
                    access_token_expires_at.to_rfc3339_opts(SecondsFormat::Secs, true)
                );
                fabro_util::printerr!(
                    printer,
                    "  Refresh expires: {}",
                    refresh_token_expires_at.to_rfc3339_opts(SecondsFormat::Secs, true)
                );
            }
            StatusRow::DevToken {
                server,
                logged_in_at,
            } => {
                fabro_util::printerr!(printer, "{server}");
                fabro_util::printerr!(printer, "  Auth: dev-token");
                fabro_util::printerr!(
                    printer,
                    "  Logged in: {}",
                    logged_in_at.to_rfc3339_opts(SecondsFormat::Secs, true)
                );
            }
        }
    }
    fabro_util::printerr!(printer, "");
    Ok(())
}

/// The empty-status message (fabro-3c11): scoped to one target it names that
/// target and the logins that DO exist — a wrong-scheme server selection
/// must not read as "not logged in to any servers". Unscoped it stays the
/// plain sentence (the store is genuinely empty).
fn empty_status_message(
    scoped_target: Option<&str>,
    env_server: Option<&str>,
    store: &AuthStore,
) -> String {
    let stored = store
        .list()
        .map(|entries| {
            entries
                .into_iter()
                .map(|(target, _)| target.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let Some(target) = scoped_target else {
        return "Not logged in to any servers.".to_string();
    };
    let scope = if stored.is_empty() {
        format!("Not logged in to {target}; the auth store is empty.")
    } else {
        format!(
            "Not logged in to {target}. Stored logins (exact server URLs): {}.",
            stored.join(", ")
        )
    };
    if env_server.is_some_and(|value| value == target) {
        return format!("{scope} The scope came from FABRO_SERVER, not a --server flag.");
    }
    scope
}

fn all_rows(store: &AuthStore, now: DateTime<Utc>) -> Result<Vec<StatusRow>> {
    Ok(store
        .list()?
        .into_iter()
        .map(|(target, entry)| status_row(&target, entry, now))
        .collect())
}

fn filter_rows(
    store: &AuthStore,
    target: &ServerTarget,
    now: DateTime<Utc>,
) -> Result<Vec<StatusRow>> {
    Ok(store
        .get(target)?
        .into_iter()
        .map(|entry| status_row(target, entry, now))
        .collect())
}

fn status_row(target: &ServerTarget, entry: AuthEntry, now: DateTime<Utc>) -> StatusRow {
    match entry {
        AuthEntry::OAuth(entry) => StatusRow::OAuth {
            server:                   target.to_string(),
            oauth_state:              oauth_state(&entry, now),
            access_token_expires_at:  entry.access_token_expires_at,
            refresh_token_expires_at: entry.refresh_token_expires_at,
            logged_in_at:             entry.logged_in_at,
            login:                    entry.subject.login,
            name:                     entry.subject.name,
            email:                    entry.subject.email,
            idp_issuer:               entry.subject.idp_issuer,
            idp_subject:              entry.subject.idp_subject,
        },
        AuthEntry::DevToken(entry) => StatusRow::DevToken {
            server:       target.to_string(),
            logged_in_at: entry.logged_in_at,
        },
    }
}

fn oauth_state(entry: &OAuthEntry, now: DateTime<Utc>) -> OAuthState {
    if entry.access_token_expires_at > now {
        OAuthState::Active
    } else if entry.refresh_token_expires_at > now {
        OAuthState::ExpiredRefreshable
    } else {
        OAuthState::Expired
    }
}

fn human_state(state: OAuthState) -> &'static str {
    match state {
        OAuthState::Active => "active",
        OAuthState::ExpiredRefreshable => "expired (refreshable)",
        OAuthState::Expired => "expired",
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use fabro_client::{AuthStore, OAuthEntry, StoredSubject};

    use super::{OAuthState, human_state, oauth_state};

    fn entry(access_offset_secs: i64, refresh_offset_secs: i64) -> OAuthEntry {
        let now = chrono::Utc::now();
        OAuthEntry {
            access_token:             "access".to_string(),
            access_token_expires_at:  now + Duration::seconds(access_offset_secs),
            refresh_token:            "refresh".to_string(),
            refresh_token_expires_at: now + Duration::seconds(refresh_offset_secs),
            subject:                  StoredSubject {
                idp_issuer:  "https://github.com".to_string(),
                idp_subject: "12345".to_string(),
                login:       "octocat".to_string(),
                name:        "The Octocat".to_string(),
                email:       "octocat@example.com".to_string(),
            },
            logged_in_at:             now,
        }
    }

    #[test]
    fn reports_active_when_access_token_is_live() {
        assert_eq!(
            oauth_state(&entry(60, 120), chrono::Utc::now()),
            OAuthState::Active
        );
    }

    #[test]
    fn reports_refreshable_when_access_is_expired_but_refresh_is_live() {
        assert_eq!(
            oauth_state(&entry(-60, 120), chrono::Utc::now()),
            OAuthState::ExpiredRefreshable
        );
    }

    #[test]
    fn reports_expired_when_both_tokens_are_expired() {
        assert_eq!(
            oauth_state(&entry(-120, -60), chrono::Utc::now()),
            OAuthState::Expired
        );
    }

    #[test]
    fn human_labels_match_expected_output() {
        assert_eq!(human_state(OAuthState::Active), "active");
        assert_eq!(
            human_state(OAuthState::ExpiredRefreshable),
            "expired (refreshable)"
        );
        assert_eq!(human_state(OAuthState::Expired), "expired");
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "test fixture writes the auth store synchronously, off the Tokio paths"
    )]
    fn store_with(server: &str) -> AuthStore {
        let dir =
            std::env::temp_dir().join(format!("fabro-auth-status-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create test dir");
        std::fs::write(
            dir.join("auth.json"),
            format!(
                r#"{{"servers":{{"{server}":{{"kind":"dev-token","token":"t","logged_in_at":"2026-10-04T00:00:00Z"}}}}}}"#
            ),
        )
        .expect("write test store");
        AuthStore::new(dir.join("auth.json"))
    }

    #[test]
    fn unscoped_empty_store_keeps_the_plain_message() {
        let empty = AuthStore::new(std::path::PathBuf::from(
            "/nonexistent-auth-store/auth.json",
        ));
        assert_eq!(
            super::empty_status_message(None, None, &empty),
            "Not logged in to any servers."
        );
    }

    #[test]
    fn scoped_empty_names_the_target_and_the_stored_logins() {
        let store = store_with("https://mirtuell.net");
        let message = super::empty_status_message(
            Some("http://mirtuell.net"),
            Some("http://mirtuell.net"),
            &store,
        );
        assert!(
            message.contains("Not logged in to http://mirtuell.net."),
            "{message}"
        );
        assert!(message.contains("https://mirtuell.net"), "{message}");
        assert!(
            message.contains("FABRO_SERVER"),
            "the env origin is named: {message}"
        );
    }

    #[test]
    fn scoped_with_nothing_stored_says_the_store_is_empty() {
        let empty = AuthStore::new(std::path::PathBuf::from(
            "/nonexistent-auth-store/auth.json",
        ));
        let message = super::empty_status_message(Some("http://x.test"), None, &empty);
        assert!(message.contains("auth store is empty"), "{message}");
    }
}
