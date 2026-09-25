/// A fork runs in its own worker and its own stage processes while the
/// source keeps running (fabro-2406: the fork must not resume into the
/// source's sandbox), and a fork worker dying without a terminal event
/// records a structured terminal failure instead of sitting silent
/// (fabro-2406 finding 2).
#[tokio::test(flavor = "multi_thread")]
async fn a_petri_fork_runs_fresh_processes_and_records_worker_crashes() {
    if host_plugin().is_none() {
        return;
    }
    let context = test_context!();
    let server = RunningServer::start().await;
    let gate = context.temp_dir.join("fork-second.gate");
    let hold = format!("while [ ! -f {} ]; do sleep 0.05; done", gate.display());
    let workspace = write_petri_workflow(
        &context,
        &format!(
            "digraph Command {{\n  graph [goal=\"Two stages for a fork\", default_max_retries=0]\n  \
             start [shape=Mdiamond]\n  exit [shape=Msquare]\n  first [shape=parallelogram, \
             script=\"echo first-done\", max_retries=0]\n  second [shape=parallelogram, \
             script=\"{hold}\", max_retries=0]\n  start -> first -> second -> exit\n}}\n"
        ),
    );
    let run_id = run_detached(&context, &server, &workspace);

    wait_for_status(&server, &run_id, &["running"]).await;
    let worker = wait_for_worker(&run_id);
    wait_until_gate_is_polled(&gate);
    eprintln!("source {run_id} holds at the second stage (worker {worker})");

    // Fork from the latest checkpoint: the boundary after `first`.
    let target = server.target();
    seed_dev_token_auth(
        &context.home_dir,
        &ServerTarget::http_url(&target).expect("the target parses"),
        TEST_DEV_TOKEN,
    );
    let fork_output = context
        .command()
        .args(["fork", "--server", &target, "--json", &run_id])
        .output()
        .expect("the fork executes");
    assert!(
        fork_output.status.success(),
        "fork failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&fork_output.stdout),
        String::from_utf8_lossy(&fork_output.stderr)
    );
    let fork_json: serde_json::Value =
        serde_json::from_slice(&fork_output.stdout).expect("the fork prints JSON");
    let fork_id = fork_json["new_run_id"]
        .as_str()
        .expect("the fork names its new run")
        .to_owned();
    eprintln!("fork {fork_id} created");

    // The fork auto-launches its own worker beside the source's
    // (starting it explicitly is refused while an engine process runs).
    wait_for_status(&server, &fork_id, &["running"]).await;
    let fork_worker = wait_for_worker(&fork_id);
    assert_ne!(
        fork_worker, worker,
        "the fork runs in its own worker, not the source's"
    );
    eprintln!("fork worker {fork_worker} launched beside source worker {worker}");

    // The fork's own stage process polls the same gate: two pollers prove
    // two live stage executions from two run workers.
    let two_pollers = Instant::now() + RUN_TIMEOUT;
    loop {
        let count = Command::new("sh")
            .arg("-c")
            .arg(format!("pgrep -f {} | wc -l", gate.display()))
            .output()
            .expect("the poller count runs");
        if String::from_utf8_lossy(&count.stdout)
            .trim()
            .parse::<usize>()
            .unwrap_or(0)
            >= 2
        {
            break;
        }
        assert!(
            Instant::now() < two_pollers,
            "the fork's stage never polled the gate beside the source's"
        );
        std::thread::sleep(POLL);
    }

    // The crash: kill the fork's worker whole process group.
    fabro_proc::sigkill_process_group(fork_worker);
    wait_for_status(&server, &fork_id, &["failed"]).await;
    let fork_state = run_json(&server, &format!("runs/{fork_id}")).await;
    let rendered = fork_state.to_string();
    assert!(
        rendered.contains("Worker exited before emitting a terminal run event"),
        "the fork's crash is a structured terminal failure: {rendered}"
    );
    eprintln!("fork crash recorded with the structured worker-exit failure");

    // The source keeps running untouched, then finishes when released.
    std::fs::write(&gate, "release\n").expect("the gate writes");
    wait_for_status(&server, &run_id, &["succeeded"]).await;
    server.shutdown();
}

/// A delete issued the moment the run reads as ended is accepted while its
/// worker is still tearing down: the server settles the run at Petri's own
/// finish, the record the view ends the run on, not at the worker's exit,
/// so the delete precheck does not refuse the run as active. The worker's
/// exit after the delete brings nothing back.
#[tokio::test(flavor = "multi_thread")]
async fn a_delete_right_after_the_run_reads_ended_is_accepted() {
    if host_plugin().is_none() {
        return;
    }
    let context = test_context!();
    let server = RunningServer::start().await;
    let workspace = write_petri_workspace(&context, "true");
    let run_id = run_detached(&context, &server, &workspace);
    let status = wait_for_status(&server, &run_id, &["succeeded", "failed"]).await;
    assert_eq!(status, "succeeded");

    let response = fabro_test::test_http_client()
        .delete(format!("{}/api/v1/runs/{run_id}", server.api_base_url))
        .bearer_auth(TEST_DEV_TOKEN)
        .send()
        .await
        .expect("the delete sends");
    let status = response.status();
    let detail = response.text().await.unwrap_or_default();
    assert_eq!(
        status,
        fabro_http::StatusCode::NO_CONTENT,
        "the delete was refused: {detail}"
    );

    let deadline = Instant::now() + RUN_TIMEOUT;
    while worker_pid(&run_id).is_some() {
        assert!(
            Instant::now() < deadline,
            "the worker of run {run_id} outlived the delete"
        );
        tokio::time::sleep(POLL).await;
    }
    let response = fabro_test::test_http_client()
        .get(format!("{}/api/v1/runs/{run_id}", server.api_base_url))
        .bearer_auth(TEST_DEV_TOKEN)
        .send()
        .await
        .expect("the request sends");
    assert_eq!(
        response.status(),
        fabro_http::StatusCode::NOT_FOUND,
        "the worker's exit brought the run back"
    );
    server.shutdown();
}
