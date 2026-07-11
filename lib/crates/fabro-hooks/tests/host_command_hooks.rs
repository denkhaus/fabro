use std::path::Path;
use std::sync::Arc;

use fabro_agent::{LocalSandbox, Sandbox};
use fabro_auth::{CredentialSource, EnvCredentialSource};
use fabro_hooks::{
    HookContext, HookDecision, HookEvent, HookExecutionContext, HookRunner, HookSettings,
    RuntimeHookDefinition, RuntimeHookType,
};
use fabro_model::Catalog;
use fabro_types::RunId;
use tokio::fs;

fn test_llm_source() -> Arc<dyn CredentialSource> {
    Arc::new(EnvCredentialSource::new())
}

fn test_catalog() -> Arc<Catalog> {
    Arc::new(Catalog::from_builtin().expect("default catalog should build"))
}

fn local_sandbox() -> Arc<dyn Sandbox> {
    Arc::new(LocalSandbox::new(
        std::env::current_dir().expect("test process should have a cwd"),
    ))
}

#[tokio::test]
async fn host_command_hook_uses_host_workdir_not_sandbox_workdir() {
    let host_work_dir =
        std::env::temp_dir().join(format!("fabro-host-hook-cwd-{}", std::process::id()));
    let _ = fs::remove_dir_all(&host_work_dir).await;
    fs::create_dir_all(&host_work_dir)
        .await
        .expect("test should create host hook cwd");
    let container_only_work_dir = Path::new("/workspace/fabro-host-hook-repro-missing");
    assert!(
        !container_only_work_dir.exists(),
        "reproduction requires a container-only cwd that does not exist on the host"
    );

    let runner = HookRunner::new(
        HookSettings {
            hooks: vec![RuntimeHookDefinition {
                name:           Some("host-marker".to_string()),
                event:          HookEvent::RunStart,
                hook_type:      Some(RuntimeHookType::Command {
                    command: "printf ran > marker.txt".to_string(),
                }),
                matcher:        None,
                blocking:       Some(true),
                timeout_ms:     Some(5000),
                sandbox:        Some(false),
                effective_name: "host-marker".to_string(),
            }],
        },
        test_llm_source(),
        test_catalog(),
    );
    let context = HookContext::new(
        HookEvent::RunStart,
        RunId::new(),
        "host-hook-cwd".to_string(),
    );

    let decision = runner
        .run(&context, local_sandbox(), HookExecutionContext {
            host_source_dir:  Some(host_work_dir.clone()),
            sandbox_work_dir: Some(container_only_work_dir.to_path_buf()),
        })
        .await;

    assert_eq!(decision, HookDecision::Proceed);
    assert_eq!(
        fs::read_to_string(host_work_dir.join("marker.txt"))
            .await
            .expect("host hook should create marker file"),
        "ran"
    );
    fs::remove_dir_all(&host_work_dir)
        .await
        .expect("test should clean up host hook cwd");
}
