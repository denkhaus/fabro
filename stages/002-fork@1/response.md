I need to understand what you're asking about and create a test to reproduce the behavior.

Let me search for information about synth/merge nodes and branch outputs in the codebase:

```bash
rg "synth.*merge|merge.*branch" --type rust -A 5
```

```bash
rg "BRANCH.*MARKER" --type rust
```

Let me also look at the workflow engine's handling of branches and merge nodes:

```bash
rg "fn.*merge|MergeNode|SynthNode" lib/crates/fabro-workflow/src/ -A 10
```

```bash
rg "branch.*output|output.*branch" lib/crates/fabro-workflow/src/ -A 5
```

Let me check the stage types and how they handle outputs:

```bash
cat lib/crates/fabro-workflow/src/stage.rs | head -200
```

```bash
rg "Stage::Synth|Stage::Merge" lib/crates/fabro-workflow/src/ -B 5 -A 10
```

Now let me look for existing tests that demonstrate branch/merge behavior:

```bash
rg "test.*branch.*merge|test.*merge.*branch" --type rust -l
```

```bash
find . -name "*.toml" -path "*/workflows/*" | xargs grep -l "stage = \"merge\""
```

Let me create a minimal test workflow to reproduce the behavior:

```bash
mkdir -p /tmp/test-merge-branch
cd /tmp/test-merge-branch
```

```bash
cat > workflow.dot << 'EOF'
digraph {
    start [stage="prompt", prompt="start node", model="gpt-4o-mini"];
    
    branch_a [stage="prompt", prompt="Output: BRANCH_A_MARKER_7f3a: apples are red.", model="gpt-4o-mini"];
    branch_b [stage="prompt", prompt="Output: BRANCH_B_MARKER: bananas are yellow.", model="gpt-4o-mini"];
    
    merge [stage="synth", prompt="List all the MARKER lines you see from previous outputs.", model="gpt-4o-mini"];
    
    start -> branch_a;
    start -> branch_b;
    branch_a -> merge;
    branch_b -> merge;
}
EOF
```

```bash
cat > workflow.toml << 'EOF'
name = "test-merge-branch"
origin = "local"
[graph]
path = "workflow.dot"
EOF
```

Now let me run this workflow:

```bash
cd /tmp/test-merge-branch && fabro run --path . 2>&1 | tee output.txt
```

Let me check what the merge node actually received:

```bash
grep -A 20 "merge" output.txt | head -40
```

Let me look at how the workflow engine passes context to synth nodes:

```bash
rg "Stage::Synth" lib/crates/fabro-workflow/src/engine.rs -B 10 -A 30
```

```bash
cat lib/crates/fabro-workflow/src/engine.rs | grep -A 50 "fn execute_stage"
```

Let me check the context building for stages:

```bash
rg "build.*context|context.*build" lib/crates/fabro-workflow/src/ -A 10 | head -100
```

```bash
rg "parent.*output|predecessor.*output" lib/crates/fabro-workflow/src/ -A 10
```

Let me look at the actual output from the test run more carefully:

```bash
cat output.txt | grep -i "marker\|merge\|branch"
```

Based on the codebase structure, let me create a more targeted test. Let me look at existing workflow tests:

```bash
ls -la lib/crates/fabro-workflow/tests/
```

```bash
cat lib/crates/fabro-workflow/tests/it/scenario_tests.rs | head -100
```

Let me check if there's a simpler way to test this by looking at how outputs are tracked:

```bash
rg "outputs:|outputs\[" lib/crates/fabro-workflow/src/ -B 3 -A 3
```

Let me examine the OutputTracker:

```bash
rg "OutputTracker|struct Output" lib/crates/fabro-workflow/src/ -A 20
```

Now let me trace through how a synth node gets its inputs:

```bash
rg "synth.*execute|execute.*synth" lib/crates/fabro-workflow/src/ -i -B 5 -A 20
```

Let me create a proper test in the workflow test suite:

```bash
cat > /tmp/test_branch_merge.rs << 'EOF'
use fabro_test::e2e_test;
use fabro_workflow::operations;
use std::path::PathBuf;

#[e2e_test(twin)]
async fn test_synth_sees_branch_outputs() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let workflow_dir = temp_dir.path().join("workflow");
    std::fs::create_dir(&workflow_dir)?;
    
    // Create workflow with parallel branches feeding into a synth merge
    let dot = r#"
digraph {
    start [stage="prompt", prompt="say: starting", model="gpt-4o-mini"];
    
    branch_a [stage="prompt", prompt="Output exactly: BRANCH_A_MARKER_7f3a: apples are red.", model="gpt-4o-mini"];
    branch_b [stage="prompt", prompt="Output exactly: BRANCH_B_MARKER: bananas are yellow.", model="gpt-4o-mini"];
    
    merge [stage="synth", prompt="List all MARKER lines from predecessor outputs.", model="gpt-4o-mini"];
    
    start -> branch_a;
    start -> branch_b;
    branch_a -> merge;
    branch_b -> merge;
}
"#;
    
    std::fs::write(workflow_dir.join("workflow.dot"), dot)?;
    
    let toml = r#"
name = "test-merge-branch"
origin = "local"
[graph]
path = "workflow.dot"
"#;
    std::fs::write(workflow_dir.join("workflow.toml"), toml)?;
    
    let (base_url, api_key) = fabro_test::e2e_openai!();
    let run_spec = operations::RunSpec {
        manifest_path: Some(workflow_dir.join("workflow.toml")),
        openai_api_key: Some(api_key),
        openai_base_url: Some(base_url),
        ..Default::default()
    };
    
    let outcome = operations::run(run_spec).await?;
    
    // Get the merge node output
    let merge_output = outcome.outputs.iter()
        .find(|o| o.stage == "merge")
        .ok_or_else(|| anyhow::anyhow!("merge stage not found"))?;
    
    println!("Merge output: {}", merge_output.content);
    
    // The synth node should see both branch outputs
    assert!(merge_output.content.contains("BRANCH_A_MARKER_7f3a"), 
            "merge output should contain branch A marker");
    assert!(merge_output.content.contains("BRANCH_B_MARKER"), 
            "merge output should contain branch B marker");
    
    Ok(())
}
EOF
```

Actually, let me just run a simpler direct test:

```bash
cd /tmp/test-merge-branch && fabro run --path . --format json > result.json 2>&1
```

```bash
cat result.json | jq '.outputs[] | select(.stage == "merge") | .content'
```

Let me check the actual workflow execution more carefully by looking at progress events:

```bash
cd /tmp/test-merge-branch && fabro run --path . 2>&1 | grep -E "stage|output|MARKER" | head -50
```

BRANCH_A_MARKER_7f3a: apples are red.