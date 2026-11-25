use std::any::{TypeId, type_name};

use fabro_api::types::{
    AskNamespace as ApiAskNamespace, LogDestination as ApiLogDestination,
    ObjectStoreSettings as ApiObjectStoreSettings, ServerNamespace as ApiServerNamespace,
    ServerSandboxProviderSettings as ApiServerSandboxProviderSettings,
    ServerSandboxProvidersSettings as ApiServerSandboxProvidersSettings,
    ServerSandboxSettings as ApiServerSandboxSettings, ServerSettings as ApiServerSettings,
};
use fabro_config::ServerSettingsBuilder;
use fabro_types::ServerSettings;
use fabro_types::settings::server::{
    LogDestination, ObjectStoreSettings, ServerSandboxProviderSettings,
    ServerSandboxProvidersSettings, ServerSandboxSettings,
};
use fabro_types::settings::{AskNamespace, ServerNamespace};

#[test]
fn server_settings_family_reuses_domain_types() {
    assert_same_type::<ApiServerSettings, ServerSettings>();
    assert_same_type::<ApiServerNamespace, ServerNamespace>();
    assert_same_type::<ApiAskNamespace, AskNamespace>();
    assert_same_type::<ApiObjectStoreSettings, ObjectStoreSettings>();
    assert_same_type::<ApiLogDestination, LogDestination>();
    assert_same_type::<ApiServerSandboxSettings, ServerSandboxSettings>();
    assert_same_type::<ApiServerSandboxProvidersSettings, ServerSandboxProvidersSettings>();
    assert_same_type::<ApiServerSandboxProviderSettings, ServerSandboxProviderSettings>();
}

#[test]
fn server_settings_json_matches_openapi_shape() {
    let settings = ServerSettingsBuilder::from_toml(
        r#"
_version = 1

[server.listen]
type = "tcp"
address = "127.0.0.1:32276"

[server.api]
url = "https://api.fabro.example.com"

[server.web]
enabled = true
url = "https://fabro.example.com"

[server.auth]
methods = ["dev-token", "github"]

[server.auth.github]
allowed_usernames = ["alice"]

[server.sandbox.providers.daytona]
enabled = false

[server.storage]
root = "/srv/fabro"

[server.logging]
destination = "stdout"

[server.integrations.github]
enabled = true
strategy = "app"
app_id = "12345"
client_id = "Iv1.abcdef"
slug = "fabro-dev"

[server.integrations.github.webhooks]
strategy = "tailscale_funnel"

"#,
    )
    .expect("settings should resolve");

    let json = serde_json::to_value(&settings).expect("server settings should serialize");
    assert_eq!(json["server"]["listen"]["type"], "tcp");
    assert_eq!(json["server"]["listen"]["address"], "127.0.0.1:32276");
    assert_eq!(json["server"]["storage"]["root"], "/srv/fabro");
    assert_eq!(json["server"]["logging"]["destination"], "stdout");
    // The `[ask]` namespace (work#1690) rides on the settings payload with
    // its key unset: absent from the map, but the namespace object itself
    // is always present (the GET /settings contract).
    assert_eq!(json["ask"]["supervisor_model"], serde_json::Value::Null);
    assert_eq!(
        json["server"]["sandbox"]["providers"]["local"]["enabled"],
        true
    );
    assert_eq!(
        json["server"]["sandbox"]["providers"]["docker"]["enabled"],
        true
    );
    assert_eq!(
        json["server"]["sandbox"]["providers"]["daytona"]["enabled"],
        false
    );
    assert!(
        json["server"].get("ip_allowlist").is_none(),
        "server settings API should not expose removed IP allowlist settings"
    );
    assert!(
        json["server"]["integrations"]["github"]["webhooks"]
            .get("ip_allowlist")
            .is_none(),
        "github webhook settings API should not expose removed IP allowlist settings"
    );
    assert!(json.get("features").is_none());

    assert!(
        json["server"].get("seeds").is_none(),
        "server settings should not expose the removed seeds mirror settings"
    );

    let round_trip: ApiServerSettings =
        serde_json::from_value(json).expect("server settings should deserialize");
    assert_eq!(round_trip, settings);

    // The supervisor model key round-trips as a plain optional string
    // (work#1690).
    let pinned = ServerSettingsBuilder::from_toml(
        r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[ask]
supervisor_model = "openai:gpt-5.6-sol"
"#,
    )
    .expect("settings with [ask] should resolve");
    assert_eq!(
        pinned.ask.supervisor_model.as_deref(),
        Some("openai:gpt-5.6-sol")
    );
    let pinned_json =
        serde_json::to_value(&pinned).expect("pinned server settings should serialize");
    assert_eq!(pinned_json["ask"]["supervisor_model"], "openai:gpt-5.6-sol");
    let pinned_round_trip: ApiServerSettings =
        serde_json::from_value(pinned_json).expect("pinned server settings should deserialize");
    assert_eq!(pinned_round_trip, pinned);
}

fn assert_same_type<T: 'static, U: 'static>() {
    assert_eq!(
        TypeId::of::<T>(),
        TypeId::of::<U>(),
        "{} should be the same type as {}",
        type_name::<T>(),
        type_name::<U>()
    );
}
