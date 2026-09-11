//! MCP servers for a workflow agent stage.
//!
//! `McpTransport::Sandbox` servers start inside the run sandbox and are
//! reached over HTTP through the sandbox's preview URL; every other transport
//! is connected as configured. The outcome of each server is reported back so
//! the stage can record it as a run event.

use std::collections::HashMap;
use std::sync::Arc;

use fabro_mcp::config::{McpServerSettings, McpTransport};
use fabro_mcp::connection_manager::McpConnectionManager;
use fabro_mcp::http_transport;
use fabro_sandbox::{RunSandbox, shell_quote};
use fabro_types::AgentMcpToolSummary;
use fabro_util::shell::shell_join;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::error::Error;

/// What became of one configured MCP server.
pub(crate) enum McpServerOutcome {
    Ready {
        tool_count: usize,
        tools:      Vec<AgentMcpToolSummary>,
    },
    Failed {
        error: String,
    },
}

pub(crate) struct McpStartup {
    pub(crate) manager:  Arc<McpConnectionManager>,
    /// One entry per configured server, in configuration order.
    pub(crate) outcomes: Vec<(String, McpServerOutcome)>,
}

/// Start every configured server and connect to it.
///
/// # Errors
///
/// Returns [`Error::Cancelled`] when `cancel_token` fires; a server that
/// fails to start or connect is reported in the outcomes instead.
pub(crate) async fn start_mcp_servers(
    sandbox: &RunSandbox,
    servers: &[McpServerSettings],
    cancel_token: &CancellationToken,
) -> Result<McpStartup, Error> {
    let mut outcomes = Vec::with_capacity(servers.len());
    let mut resolved = Vec::with_capacity(servers.len());
    for config in servers {
        if cancel_token.is_cancelled() {
            return Err(Error::Cancelled);
        }
        match &config.transport {
            McpTransport::Sandbox {
                protocol,
                command,
                port,
                env,
            } => {
                match start_sandbox_mcp_server(sandbox, command, *port, env, cancel_token).await? {
                    Ok((url, headers)) => {
                        match http_transport::sandbox_mcp_http_url(*protocol, &url) {
                            Ok(url) => {
                                info!(
                                    server = %config.name,
                                    url = %url,
                                    "Sandbox MCP server started, connecting via HTTP"
                                );
                                resolved.push(McpServerSettings {
                                    name:                 config.name.clone(),
                                    transport:            McpTransport::Http {
                                        protocol: *protocol,
                                        url,
                                        headers,
                                    },
                                    current_dir:          config.current_dir.clone(),
                                    clear_env:            config.clear_env,
                                    startup_timeout_secs: config.startup_timeout_secs,
                                    tool_timeout_secs:    config.tool_timeout_secs,
                                });
                            }
                            Err(error) => {
                                outcomes.push((config.name.clone(), McpServerOutcome::Failed {
                                    error: error.to_string(),
                                }));
                            }
                        }
                    }
                    Err(error) => {
                        warn!(server = %config.name, error = %error, "Failed to start sandbox MCP server");
                        outcomes.push((config.name.clone(), McpServerOutcome::Failed { error }));
                    }
                }
            }
            _ => resolved.push(config.clone()),
        }
    }

    let mut manager = McpConnectionManager::new();
    for (server_name, result) in manager.start_servers(&resolved).await {
        let outcome = match result {
            Ok(tool_count) => McpServerOutcome::Ready {
                tool_count,
                tools: manager
                    .tool_summaries_for_server(&server_name)
                    .into_iter()
                    .map(|(name, original_name)| AgentMcpToolSummary {
                        name,
                        original_name,
                    })
                    .collect(),
            },
            Err(error) => McpServerOutcome::Failed {
                error: error.to_string(),
            },
        };
        outcomes.push((server_name, outcome));
    }

    Ok(McpStartup {
        manager: Arc::new(manager),
        outcomes,
    })
}

/// Start an MCP server inside the sandbox and return `(url, headers)` for
/// the HTTP connection.
///
/// The outer `Result` is cancellation (the running MCP process group is
/// terminated before returning). The inner `Result` is a non-fatal startup
/// failure the caller reports as `agent.mcp.failed`.
async fn start_sandbox_mcp_server(
    sandbox: &RunSandbox,
    command: &[String],
    port: u16,
    env: &HashMap<String, String>,
    cancel_token: &CancellationToken,
) -> Result<Result<(String, HashMap<String, String>), String>, Error> {
    let launch_script = sandbox_mcp_launch_script(command);
    let env_ref = if env.is_empty() { None } else { Some(env) };

    if cancel_token.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let launch_result = match sandbox
        .exec_command(
            &launch_script,
            30_000,
            None,
            env_ref,
            Some(cancel_token.child_token()),
        )
        .await
    {
        Ok(result) => result,
        Err(error) => {
            if cancel_token.is_cancelled() {
                return Err(Error::Cancelled);
            }
            return Ok(Err(format!(
                "Failed to launch MCP server: {}",
                error.display_with_causes()
            )));
        }
    };

    let pid = launch_result.stdout.trim().to_string();
    info!(pid = %pid, port, "MCP server process launched in sandbox");

    // Wait for the server to start listening on the port.
    let poll_cmd = format!(
        "for i in $(seq 1 30); do ss -tln | grep -q ':{port} ' && echo ready && exit 0; sleep 1; done; echo timeout"
    );
    let poll_result = sandbox
        .exec_command(
            &poll_cmd,
            60_000,
            None,
            None,
            Some(cancel_token.child_token()),
        )
        .await;

    if cancel_token.is_cancelled() {
        kill_mcp_pid(sandbox, &pid).await;
        return Err(Error::Cancelled);
    }

    let poll_result = match poll_result {
        Ok(result) => result,
        Err(error) => {
            return Ok(Err(format!(
                "Failed to poll MCP server readiness: {}",
                error.display_with_causes()
            )));
        }
    };

    if poll_result.stdout.trim() != "ready" {
        let stderr = sandbox
            .exec_command(
                "cat /tmp/mcp_server_stderr.log 2>/dev/null | tail -20",
                10_000,
                None,
                None,
                Some(cancel_token.child_token()),
            )
            .await
            .map(|result| result.stdout)
            .unwrap_or_default();
        return Ok(Err(format!(
            "MCP server did not start listening on port {port} within 30s. stderr:\n{stderr}"
        )));
    }

    // The preview URL for the port, or localhost for local sandboxes.
    let preview = match sandbox.get_preview_url(port).await {
        Ok(preview) => preview,
        Err(error) => return Ok(Err(error.display_with_causes())),
    };

    if cancel_token.is_cancelled() {
        kill_mcp_pid(sandbox, &pid).await;
        return Err(Error::Cancelled);
    }

    if let Some(url_and_headers) = preview {
        Ok(Ok(url_and_headers))
    } else {
        info!(port, "No preview URL available, using localhost");
        Ok(Ok((format!("http://localhost:{port}"), HashMap::new())))
    }
}

fn sandbox_mcp_launch_script(command: &[String]) -> String {
    let command_source = match command {
        // Sandbox MCP `script` entries resolve to this exact argv shape. The
        // surrounding launcher is already the provider-selected Bash, so
        // evaluate the source in that process instead of PATH-resolving a
        // second interpreter. Grouping keeps the log redirections scoped to
        // the whole script, including multi-command and trailing-comment
        // forms.
        [interpreter, flag, source] if interpreter == "bash" && flag == "-c" => {
            format!("{{\n{source}\n}}")
        }
        _ => shell_join(command),
    };
    let inner =
        format!("{command_source} > /tmp/mcp_server_stdout.log 2>/tmp/mcp_server_stderr.log");
    format!(
        "setsid \"$BASH\" -c {quoted} </dev/null >/dev/null 2>&1 &\necho $!",
        quoted = shell_quote(&inner)
    )
}

/// Best-effort kill of a sandbox MCP server process group, used when startup
/// is cancelled after the detached process has been spawned.
async fn kill_mcp_pid(sandbox: &RunSandbox, pid: &str) {
    let pid = pid.trim();
    if pid.is_empty() {
        return;
    }
    let script =
        format!("kill -TERM -{pid} 2>/dev/null; sleep 1; kill -KILL -{pid} 2>/dev/null; true");
    if let Err(error) = sandbox.exec_command(&script, 5_000, None, None, None).await {
        warn!(pid, error = %error.display_with_causes(), "Failed to kill MCP server process group during cancellation");
    }
}

#[cfg(test)]
mod tests {
    use super::sandbox_mcp_launch_script;

    #[test]
    fn launch_script_evaluates_bash_c_source_in_place() {
        let script = sandbox_mcp_launch_script(&[
            "bash".to_string(),
            "-c".to_string(),
            "echo hi # trailing comment".to_string(),
        ]);

        assert!(script.starts_with("setsid \"$BASH\" -c "));
        assert!(script.contains("echo hi # trailing comment\n}"));
        assert!(script.ends_with("&\necho $!"));
    }

    #[test]
    fn launch_script_quotes_other_commands() {
        let script = sandbox_mcp_launch_script(&[
            "python3".to_string(),
            "server.py".to_string(),
            "--name".to_string(),
            "it's".to_string(),
        ]);

        assert!(script.contains("python3 server.py --name"));
        assert!(script.contains("/tmp/mcp_server_stderr.log"));
    }
}
