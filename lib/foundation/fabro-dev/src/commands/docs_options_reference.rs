use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use fabro_options_metadata::{OptionField, OptionSet};

use super::{markdown_cell, replace_generated_region};

const OPTIONS_REFERENCE_PATH: &str = "docs/public/reference/user-configuration.mdx";
const FENCE_START: &str = "{/* generated:options */}";
const FENCE_END: &str = "{/* /generated:options */}";

#[expect(
    clippy::print_stdout,
    clippy::disallowed_methods,
    reason = "dev generator reports the generated docs path directly and intentionally uses sync filesystem I/O"
)]
pub(crate) fn docs_options_reference_root(root: &Path, check: bool) -> Result<()> {
    let path = root.join(OPTIONS_REFERENCE_PATH);
    let current =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let generated = render_options_reference();
    let updated = replace_generated_region(
        &current,
        &generated,
        OPTIONS_REFERENCE_PATH,
        FENCE_START,
        FENCE_END,
    )?;

    if check {
        if current != updated {
            bail!("{OPTIONS_REFERENCE_PATH} is stale; run `cargo dev docs refresh`");
        }
        println!("{OPTIONS_REFERENCE_PATH} is up to date.");
        return Ok(());
    }

    if current != updated {
        std::fs::write(&path, updated).with_context(|| format!("writing {}", path.display()))?;
    }
    println!("Generated {OPTIONS_REFERENCE_PATH}.");
    Ok(())
}

struct Section {
    path:    &'static str,
    set:     OptionSet,
    example: &'static str,
}

impl Section {
    fn of<T>(path: &'static str, example: &'static str) -> Self
    where
        T: fabro_options_metadata::OptionsMetadata + 'static,
    {
        Self {
            path,
            set: OptionSet::of::<T>(),
            example,
        }
    }
}

fn render_options_reference() -> String {
    let mut output = String::new();
    render_manual_cli_target(&mut output);
    render_manual_llm_catalog(&mut output);

    for section in metadata_sections() {
        render_section(&mut output, &section);
    }

    render_manual_mcp(&mut output);
    output.trim_end().to_string()
}

fn metadata_sections() -> Vec<Section> {
    vec![
        Section::of::<fabro_config::CliUpdatesLayer>(
            "[cli.updates]",
            r"[cli.updates]
check = true",
        ),
        Section::of::<fabro_config::CliOutputLayer>(
            "[cli.output]",
            r#"[cli.output]
format = "text"
verbosity = "verbose""#,
        ),
        Section::of::<fabro_config::CliExecLayer>(
            "[cli.exec]",
            r"[cli.exec]
prevent_idle_sleep = true",
        ),
        Section::of::<fabro_config::CliExecModelLayer>(
            "[cli.exec.model]",
            r#"[cli.exec.model]
provider = "anthropic"
name = "claude-opus-4-6""#,
        ),
        Section::of::<fabro_config::CliExecAgentLayer>(
            "[cli.exec.agent]",
            r#"[cli.exec.agent]
permissions = "read-write""#,
        ),
        Section::of::<fabro_config::RunModelLayer>(
            "[run.model]",
            r#"[run.model]
provider = "anthropic"
name = "claude-sonnet-4-5"

[run.model.fallbacks]
"claude-sonnet-4-5" = ["openrouter:kimi-k3", "gpt-terra"]"#,
        ),
        Section::of::<fabro_config::CliLoggingLayer>(
            "[cli.logging]",
            r#"[cli.logging]
level = "info""#,
        ),
        Section::of::<fabro_config::GitAuthorLayer>(
            "[run.git.author]",
            r#"[run.git.author]
name = "fabro-bot"
email = "fabro-bot@company.com""#,
        ),
        Section::of::<fabro_config::RunPullRequestLayer>(
            "[run.pull_request]",
            r"[run.pull_request]
enabled = true",
        ),
        Section::of::<fabro_config::RunAgentLayer>(
            "[run.agent]",
            r"[run.agent]
fabro_tools = true",
        ),
    ]
}

fn render_section(output: &mut String, section: &Section) {
    output.push_str("## `");
    output.push_str(section.path);
    output.push_str("`\n\n");

    if let Some(doc) = section.set.documentation() {
        output.push_str(&normalize_doc(doc));
        output.push_str("\n\n");
    }

    output.push_str("```toml title=\"settings.toml\"\n");
    output.push_str(section.example);
    output.push_str("\n```\n\n");
    render_field_table(output, section.set.fields());
}

fn render_field_table(output: &mut String, fields: BTreeMap<String, OptionField>) {
    output.push_str("| Key | Type / values | Default | Description |\n");
    output.push_str("|---|---|---|---|\n");
    for (name, field) in fields {
        output.push_str("| `");
        output.push_str(&name);
        output.push_str("` | ");
        output.push_str(&field_type(&field));
        output.push_str(" | ");
        output.push_str(field.default.unwrap_or("None"));
        output.push_str(" | ");
        output.push_str(&markdown_cell(
            field.doc.unwrap_or("TODO: add settings help text."),
        ));
        output.push_str(" |\n");
    }
    output.push('\n');
}

fn field_type(field: &OptionField) -> String {
    if let Some(possible_values) = field
        .possible_values
        .as_ref()
        .filter(|values| !values.is_empty())
    {
        possible_values
            .iter()
            .map(|value| format!("`{}`", value.name))
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        field
            .value_type
            .map_or_else(|| "inferred".to_string(), markdown_cell)
    }
}

fn render_manual_cli_target(output: &mut String) {
    output.push_str(
        r#"## `[cli.target]`

Connection info for commands that target a remote Fabro server.

```toml title="settings.toml"
[cli.target]
type = "http"
url = "https://fabro.example.com/api/v1"
```

| Key | Type / values | Default | Description |
|---|---|---|---|
| `type` | `"http"` \| `"unix"` | None | Explicit transport selection. |
| `url` | string | None | Required for `type = "http"`; the API base URL. |
| `path` | string | None | Required for `type = "unix"`; the absolute Unix socket path. |

"#,
    );
}

fn render_manual_llm_catalog(output: &mut String) {
    output.push_str(
        r#"## `[llm]`

The `[llm]` table is a [lithos-llm](https://docs.rs/lithos-llm) catalog
overlay. Fabro builds its model catalog from three layers: the lithos built-in
providers and models, Fabro's policy layer, and this table. Later layers win;
tables merge key by key and every other value replaces. Fabro does not
interpret the table itself. lithos validates it when the catalog is built, and
rejects unknown provider or model fields.

Fabro-specific policy lives under `metadata.fabro` on a provider or model.
lithos carries that namespace verbatim.

```toml title="settings.toml"
[llm.providers.proxy]
display_name = "Acme Gateway"
adapter = "openai-compatible"
codec = "openai-chat"
base_url = "https://llm-gateway.example.com/v1"
auth = { type = "bearer" }
priority = 50
aliases = ["gateway"]
default_model = "team-code-large"

[llm.providers.proxy.metadata.fabro]
enabled = true
agent_profile = "anthropic"
credentials = ["env:ACME_GATEWAY_API_KEY", "vault:ACME_GATEWAY_API_KEY"]

[llm.providers.proxy.metadata.fabro.extra_headers]
x-portkey-api-key = "{{ secrets.PORTKEY_API_KEY }}"
x-portkey-config = "@bedrock-prod"

[llm.providers.proxy.models."team-code-large"]
display_name = "Team Code Large"
aliases = ["team-code"]
api_model = "provider-wire-model-name"
limits = { context_tokens = 200000, max_output_tokens = 32000 }
capabilities = { text = true, tools = true, reasoning = true, caching = true, reasoning_effort = { low = true, medium = true, high = true } }
protocol_options = { reasoning_effort_levels = true }
pricing = { input_usd_micros_per_million = 1500000, output_usd_micros_per_million = 8000000, cached_input_usd_micros_per_million = 300000 }

[llm.providers.proxy.models."team-code-large".metadata.fabro]
family = "team-code"
small_default = true
estimated_output_tps = 80
```

## `[llm.providers.<id>]`

Define or override an LLM provider. The keys are the lithos provider record.

| Key | Type / values | Default | Description |
|---|---|---|---|
| `display_name` | string | required for new providers | Human-readable provider name. |
| `adapter` | string | required for new providers | lithos adapter id: `anthropic`, `openai`, `gemini`, `openai-compatible`, or `bedrock`. |
| `codec` | string | required for new providers | Wire codec: `anthropic-messages`, `openai-responses`, `openai-chat`, `gemini-generate`, or `bedrock-converse`. |
| `base_url` | string | required for new providers | Provider API base URL. The `openai-compatible` adapter appends `/v1/chat/completions` unless the URL already ends in a version segment. |
| `auth` | table | required for new providers | Auth scheme: `{ type = "bearer" }`, `{ type = "header", name = "x-api-key" }`, `{ type = "headers" }`, `{ type = "none" }`, or `{ type = "aws" }`. |
| `priority` | integer | `0` | Higher-priority ready providers win unqualified model and default selection. |
| `aliases` | array<string> | `[]` | Additional provider names accepted by model routing and fallback config. |
| `default_model` | string | None | The provider's default model id. |
| `allow_passthrough` | boolean | `false` | Whether `provider/model` selectors may name models the catalog does not list. |
| `default_headers` | table | `{}` | Literal headers attached to every request. Secret-bearing headers belong in `metadata.fabro.extra_headers`. |

## `[llm.providers.<id>.metadata.fabro]`

Fabro's provider policy. Every key is optional.

| Key | Type / values | Default | Description |
|---|---|---|---|
| `enabled` | boolean | `true` | Set `false` to hide a provider from Fabro. Several built-in providers ship disabled. |
| `agent_profile` | `"anthropic"` \| `"openai"` \| `"gemini"` \| `"kimi"` \| `"gpt56"` | derived from `adapter` | Agent profile for models on this provider. |
| `api_key_url` | string | None | Where an operator obtains an API key. |
| `credentials` | array<string> | `[]` | Ordered credential refs: `vault:<NAME>`, `env:<NAME>`, or `aws_sigv4`. The first that resolves wins. |
| `extra_headers` | table | `{}` | Extra request headers. Values are literal text or `{{ secrets.NAME }}` interpolation strings resolved against the vault. |

## `[llm.providers.<provider>.models.<model-id>]`

Define or override one provider's offering of a model. The table key is the
model id Fabro users reference. An offering's identity is the pair
`(provider, model id)`, so different providers may use the same id and
aliases. `api_model` is the string sent to the provider and defaults to the id.

| Key | Type / values | Default | Description |
|---|---|---|---|
| `display_name` | string | required for new models | Human-readable model name. |
| `aliases` | array<string> | `[]` | Additional selectors. Aliases may repeat across providers. |
| `api_model` | string | model id | Wire model identifier sent to this provider. |
| `limits` | `{ context_tokens, max_output_tokens }` | None | Token limits. |
| `capabilities` | table | unknown | Per-capability `true`, `false`, or `"unknown"`: `text`, `images`, `audio`, `documents`, `tools`, `reasoning`, `caching`, `cache_routing`, `sampling`, plus `tool_choice = { required, named }`, `response_format = { json_object, json_schema }`, `reasoning_effort = { minimal, low, medium, high, xhigh, max }`, and `speed = { fast, balanced, economical }`. |
| `protocol_options` | table | `{}` | Encoding flags: `reasoning_effort_levels`, `cache_breakpoints`, `system_turns`. |
| `pricing` | table | None | USD micros per million tokens: `input_usd_micros_per_million`, `output_usd_micros_per_million`, `cached_input_usd_micros_per_million`, `cache_write_usd_micros_per_million`, plus optional `long_context` and `speed` tiers. |

## `[llm.providers.<provider>.models.<model-id>.metadata.fabro]`

Fabro's model policy. Every key is optional.

| Key | Type / values | Default | Description |
|---|---|---|---|
| `enabled` | boolean | `true` | Set `false` to hide a model from Fabro. |
| `agent_profile` | profile name | provider profile | Agent profile override for this model. |
| `family` | string | model id | Family label for display and grouping. |
| `training` | string | None | Training data cutoff label. |
| `knowledge_cutoff` | string | None | Public knowledge cutoff label. |
| `estimated_output_tps` | number | None | Estimated output tokens per second. |
| `small_default` | boolean | `false` | Preferred for small utility calls such as generated run titles. |
| `probe` | boolean | `false` | Preferred for provider connectivity probes. |
| `reasoning_by_default` | boolean | reasoning models with effort levels: `true` | Whether requests reason when no `reasoning_effort` is supplied. |

"#,
    );
}

fn render_manual_mcp(output: &mut String) {
    output.push_str(
        r#"## `[run.agent.mcps.<name>]`

Configure MCP servers for workflow agents. For `fabro exec`-only MCPs, use `[cli.exec.agent.mcps.<name>]` with the same shape.

```toml title="settings.toml"
[run.agent.mcps.filesystem]
type = "stdio"
command = ["npx", "-y", "@modelcontextprotocol/server-filesystem", "/workspace"]
startup_timeout = "15s"
tool_timeout = "90s"
```

| Key | Type / values | Default | Description |
|---|---|---|---|
| `type` | `"stdio"` \| `"http"` \| `"sandbox"` | None | MCP transport type. |
| `command` | array<string> | None | Command and arguments for `stdio` or `sandbox` transports. |
| `script` | string | None | Shell script alternative to `command` for process-launching transports. A `stdio` script runs on the host through `sh -c`; a `sandbox` script is evaluated inside the sandbox by non-login Bash. |
| `url` | string | None | Remote MCP URL for `http` transport. |
| `port` | integer | None | Sandbox port for `sandbox` transport. |
| `env` | table | `{}` | Additional environment variables for process-launching transports. |
| `headers` | table | `{}` | HTTP headers for `http` transport. |
| `startup_timeout` | duration | `"10s"` | Max duration for startup and MCP handshake. |
| `tool_timeout` | duration | `"60s"` | Max duration for a single MCP tool call. |

See [MCP](/agents/mcp) for transport-specific examples.
"#,
    );
}

fn normalize_doc(doc: &str) -> String {
    doc.trim().trim_end_matches('.').to_string()
}
