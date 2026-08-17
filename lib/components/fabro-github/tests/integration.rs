use std::error::Error as _;

use fabro_github::{
    GitHubAppCredentials, GitHubContext, GitHubCredentials, GitHubRepositoryReader,
    InstallationToken, RepositoryReadError, branch_head_sha, close_pull_request,
    create_installation_access_token_for_pr, create_pull_request, enable_auto_merge,
    get_pull_request, merge_pull_request, resolve_authenticated_url, sign_app_jwt,
};
use fabro_test::{GitHubAppOptions, GitHubAppState, TwinGitHub};
use fabro_types::GitHubRepositorySlug;
use fabro_types::settings::run::MergeStrategy;

const TEST_RSA_KEY: &str = include_str!("../src/testdata/rsa_private.pem");
const HEAD_SHA: &str = "1111111111111111111111111111111111111111";
const TAG_SHA: &str = "2222222222222222222222222222222222222222";
const UPPER_SHA: &str = "ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD";

fn github_credentials() -> GitHubCredentials {
    GitHubCredentials::App(GitHubAppCredentials {
        app_id:          "42".to_string(),
        private_key_pem: TEST_RSA_KEY.to_string(),
        slug:            Some("test-app".to_string()),
    })
}

fn standard_app_state() -> GitHubAppState {
    let mut state = GitHubAppState::new();
    state.register_app(GitHubAppOptions {
        app_id:          "42".into(),
        slug:            "test-app".into(),
        owner_login:     "acme".into(),
        public:          true,
        private_key_pem: TEST_RSA_KEY.into(),
        webhook_secret:  None,
    });
    state.add_installation("42", "acme", vec!["widgets".into()], false);
    state.add_repository(
        "acme",
        "widgets",
        vec!["main".into(), "feature".into()],
        false,
    );
    state.set_repository_ref("acme", "widgets", "heads/main", HEAD_SHA);
    state.set_repository_ref("acme", "widgets", "heads/release", HEAD_SHA);
    state.set_repository_ref("acme", "widgets", "tags/release", TAG_SHA);
    state.set_repository_ref("acme", "widgets", "heads/fabro/run/123", HEAD_SHA);
    state.set_repository_ref("acme", "widgets", "heads/uppercase", UPPER_SHA);
    state.add_repository_file(
        "acme",
        "widgets",
        HEAD_SHA,
        ".fabro/workflows/build/workflow.toml",
        b"name = \"build\"\n".to_vec(),
    );
    state.add_repository_file(
        "acme",
        "widgets",
        HEAD_SHA,
        "dir/hello world#.txt",
        b"hello\n".to_vec(),
    );
    state.add_repository_file("acme", "widgets", HEAD_SHA, "invalid.txt", vec![0xff, 0xfe]);
    state.add_repository_file("acme", "widgets", HEAD_SHA, "empty.txt", Vec::new());
    state.add_repository_file(
        "acme",
        "widgets",
        HEAD_SHA,
        "five-bytes.txt",
        b"12345".to_vec(),
    );
    state.add_repository_file(
        "acme",
        "widgets",
        HEAD_SHA,
        "nested/a b/%/é.toml",
        "unicode: 🦀\n".as_bytes().to_vec(),
    );
    state
}

fn repository() -> GitHubRepositorySlug {
    GitHubRepositorySlug::try_new("acme/widgets").expect("test repository slug should be valid")
}

#[fabro_macros::e2e_test(twin)]
async fn create_and_get_pull_request() {
    let twin = TwinGitHub::start(standard_app_state()).await;
    let creds = github_credentials();
    let ctx = &GitHubContext::new(&creds, &twin.base_url);

    let created = create_pull_request(
        ctx,
        "acme",
        "widgets",
        "main",
        "feature",
        "Add widgets",
        "PR body",
        false,
    )
    .await
    .unwrap();

    let pr = get_pull_request(ctx, "acme", "widgets", created.number)
        .await
        .unwrap();

    assert_eq!(pr.title, "Add widgets");
    assert_eq!(pr.state, "open");
    assert_eq!(pr.head.ref_name, "feature");
    assert_eq!(pr.base.ref_name, "main");

    twin.shutdown().await;
}

#[fabro_macros::e2e_test(twin)]
async fn create_merge_and_verify_state() {
    let twin = TwinGitHub::start(standard_app_state()).await;
    let creds = github_credentials();
    let ctx = &GitHubContext::new(&creds, &twin.base_url);

    let created = create_pull_request(
        ctx, "acme", "widgets", "main", "feature", "Merge me", "PR body", false,
    )
    .await
    .unwrap();

    merge_pull_request(
        ctx,
        "acme",
        "widgets",
        created.number,
        MergeStrategy::Squash,
    )
    .await
    .unwrap();

    let pr = get_pull_request(ctx, "acme", "widgets", created.number)
        .await
        .unwrap();

    assert_eq!(pr.state, "closed");
    assert_eq!(pr.mergeable, Some(false));

    twin.shutdown().await;
}

#[fabro_macros::e2e_test(twin)]
async fn create_close_and_verify_state() {
    let twin = TwinGitHub::start(standard_app_state()).await;
    let creds = github_credentials();

    let ctx = &GitHubContext::new(&creds, &twin.base_url);

    let created = create_pull_request(
        ctx, "acme", "widgets", "main", "feature", "Close me", "PR body", false,
    )
    .await
    .unwrap();

    close_pull_request(ctx, "acme", "widgets", created.number)
        .await
        .unwrap();

    let pr = get_pull_request(ctx, "acme", "widgets", created.number)
        .await
        .unwrap();

    assert_eq!(pr.state, "closed");

    twin.shutdown().await;
}

#[fabro_macros::e2e_test(twin)]
async fn enable_auto_merge_persists() {
    let twin = TwinGitHub::start(standard_app_state()).await;
    let creds = github_credentials();

    let ctx = &GitHubContext::new(&creds, &twin.base_url);

    let created = create_pull_request(
        ctx,
        "acme",
        "widgets",
        "main",
        "feature",
        "Auto merge me",
        "PR body",
        false,
    )
    .await
    .unwrap();

    enable_auto_merge(
        ctx,
        "acme",
        "widgets",
        &created.node_id,
        MergeStrategy::Squash,
    )
    .await
    .unwrap();

    let GitHubCredentials::App(app_creds) = &creds else {
        panic!("expected app credentials");
    };
    let jwt = sign_app_jwt(&app_creds.app_id, &app_creds.private_key_pem).unwrap();
    let client = fabro_test::test_http_client();
    let token =
        create_installation_access_token_for_pr(&client, &jwt, "acme", "widgets", &twin.base_url)
            .await
            .unwrap();

    let detail: serde_json::Value = fabro_test::test_http_client()
        .get(format!(
            "{}/repos/acme/widgets/pulls/{}",
            twin.base_url, created.number
        ))
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "fabro")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(
        detail["auto_merge"]["merge_method"].as_str(),
        Some("SQUASH")
    );

    twin.shutdown().await;
}

#[fabro_macros::e2e_test(twin)]
async fn resolve_authenticated_url_embeds_token() {
    let twin = TwinGitHub::start(standard_app_state()).await;
    let creds = github_credentials();

    let url = resolve_authenticated_url(
        &GitHubContext::new(&creds, &twin.base_url),
        "https://github.com/acme/widgets.git",
    )
    .await
    .unwrap();

    assert!(url.raw_string().starts_with("https://x-access-token:ghs_"));
    assert!(url.raw_string().contains("github.com/acme/widgets.git"));
    assert!(
        url.redacted_string()
            .starts_with("https://x-access-token:****@")
    );

    twin.shutdown().await;
}

#[fabro_macros::e2e_test(twin)]
async fn resolve_authenticated_url_errors_on_non_github_url() {
    let twin = TwinGitHub::start(standard_app_state()).await;
    let creds = github_credentials();

    let error = resolve_authenticated_url(
        &GitHubContext::new(&creds, &twin.base_url),
        "https://gitlab.com/foo/bar",
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("Not a GitHub HTTPS URL"));

    twin.shutdown().await;
}

#[fabro_macros::e2e_test(twin)]
async fn repository_reader_reuses_one_app_token_for_ref_and_file_reads() {
    let twin = TwinGitHub::start(standard_app_state()).await;
    assert_eq!(twin.active_token_count().await, 0);
    let creds = github_credentials();
    let base_url = format!("{}/", twin.base_url);
    let reader = GitHubRepositoryReader::open(
        &GitHubContext::with_http_client(&creds, &base_url, fabro_test::test_http_client()),
        &repository(),
    )
    .await
    .unwrap();
    assert_eq!(twin.active_token_count().await, 1);

    assert_eq!(reader.resolve_commit("heads/main").await.unwrap(), HEAD_SHA);
    assert_eq!(
        reader
            .read_utf8_file_at(HEAD_SHA, ".fabro/workflows/build/workflow.toml", 1024)
            .await
            .unwrap(),
        "name = \"build\"\n"
    );
    assert_eq!(
        reader
            .read_utf8_file_at(HEAD_SHA, "dir/hello world#.txt", 1024)
            .await
            .unwrap(),
        "hello\n"
    );
    let unicode_contents = "unicode: 🦀\n";
    assert_eq!(
        reader
            .read_utf8_file_at(HEAD_SHA, "nested/a b/%/é.toml", unicode_contents.len(),)
            .await
            .unwrap(),
        unicode_contents
    );
    assert!(matches!(
        reader
            .read_utf8_file_at(HEAD_SHA, "nested/a b/%/é.toml", unicode_contents.len() - 1,)
            .await,
        Err(RepositoryReadError::BodyTooLarge { .. })
    ));
    assert_eq!(twin.active_token_count().await, 1);

    twin.shutdown().await;
}

#[fabro_macros::e2e_test(twin)]
async fn repository_reader_preserves_exact_ref_namespaces() {
    let twin = TwinGitHub::start(standard_app_state()).await;
    let creds = github_credentials();
    let reader = GitHubRepositoryReader::open(
        &GitHubContext::with_http_client(&creds, &twin.base_url, fabro_test::test_http_client()),
        &repository(),
    )
    .await
    .unwrap();

    assert_eq!(
        reader.resolve_commit("heads/release").await.unwrap(),
        HEAD_SHA
    );
    assert_eq!(
        reader.resolve_commit("tags/release").await.unwrap(),
        TAG_SHA
    );
    assert_eq!(
        reader.resolve_commit("heads/fabro/run/123").await.unwrap(),
        HEAD_SHA
    );
    assert_eq!(reader.resolve_commit(HEAD_SHA).await.unwrap(), HEAD_SHA);
    assert_eq!(
        reader.resolve_commit("heads/uppercase").await.unwrap(),
        UPPER_SHA.to_ascii_lowercase()
    );
    assert!(matches!(
        reader.resolve_commit("heads/missing").await,
        Err(RepositoryReadError::RevisionNotFound)
    ));

    twin.shutdown().await;
}

#[fabro_macros::e2e_test(twin)]
async fn repository_reader_classifies_file_failures_without_retaining_bytes() {
    let twin = TwinGitHub::start(standard_app_state()).await;
    let creds = github_credentials();
    let reader = GitHubRepositoryReader::open(
        &GitHubContext::with_http_client(&creds, &twin.base_url, fabro_test::test_http_client()),
        &repository(),
    )
    .await
    .unwrap();

    assert!(matches!(
        reader
            .read_utf8_file_at(HEAD_SHA, ".fabro/workflows/build/workflow.toml", 4)
            .await,
        Err(RepositoryReadError::BodyTooLarge { max_bytes: 4 })
    ));
    let error = reader
        .read_utf8_file_at(HEAD_SHA, "invalid.txt", 16)
        .await
        .unwrap_err();
    assert!(matches!(error, RepositoryReadError::InvalidUtf8 { .. }));
    assert!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<std::str::Utf8Error>())
            .is_some()
    );
    assert!(!format!("{error:?}").contains("255"));
    let directory_error = reader
        .read_utf8_file_at(HEAD_SHA, "dir", 1024)
        .await
        .unwrap_err();
    assert!(matches!(
        directory_error,
        RepositoryReadError::ContentNotFile
    ));
    assert!(!format!("{directory_error:?}").contains("[]"));
    assert!(matches!(
        reader
            .read_utf8_file_at(HEAD_SHA, "missing.txt", 1024)
            .await,
        Err(RepositoryReadError::ContentNotFound)
    ));
    assert_eq!(
        reader
            .read_utf8_file_at(HEAD_SHA, "five-bytes.txt", 5)
            .await
            .unwrap(),
        "12345"
    );
    assert!(matches!(
        reader
            .read_utf8_file_at(HEAD_SHA, "five-bytes.txt", 4)
            .await,
        Err(RepositoryReadError::BodyTooLarge { max_bytes: 4 })
    ));
    assert_eq!(
        reader
            .read_utf8_file_at(HEAD_SHA, "empty.txt", 0)
            .await
            .unwrap(),
        ""
    );

    twin.shutdown().await;
}

#[fabro_macros::e2e_test(twin)]
async fn static_repository_credentials_do_not_mint_tokens() {
    let mut state = standard_app_state();
    let static_token = state.generate_access_token(
        "42",
        1,
        vec!["widgets".to_string()],
        serde_json::json!({ "contents": "read" }),
    );
    let twin = TwinGitHub::start(state).await;
    let initial_tokens = twin.active_token_count().await;

    for credentials in [
        GitHubCredentials::Pat(static_token.clone()),
        GitHubCredentials::Installation(InstallationToken {
            token:      static_token.clone(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        }),
    ] {
        let reader = GitHubRepositoryReader::open(
            &GitHubContext::with_http_client(
                &credentials,
                &twin.base_url,
                fabro_test::test_http_client(),
            ),
            &repository(),
        )
        .await
        .unwrap();
        assert_eq!(reader.resolve_commit("heads/main").await.unwrap(), HEAD_SHA);
        assert_eq!(
            reader
                .read_utf8_file_at(HEAD_SHA, "five-bytes.txt", 5)
                .await
                .unwrap(),
            "12345"
        );
    }

    assert_eq!(twin.active_token_count().await, initial_tokens);
    twin.shutdown().await;
}

#[fabro_macros::e2e_test(twin)]
async fn expired_installation_token_keeps_credential_error_source() {
    let twin = TwinGitHub::start(standard_app_state()).await;
    let credentials = GitHubCredentials::Installation(InstallationToken {
        token:      "ghs_expired".to_string(),
        expires_at: chrono::Utc::now() - chrono::Duration::minutes(1),
    });

    let error = GitHubRepositoryReader::open(
        &GitHubContext::with_http_client(
            &credentials,
            &twin.base_url,
            fabro_test::test_http_client(),
        ),
        &repository(),
    )
    .await
    .err()
    .expect("expired installation token should fail");
    let RepositoryReadError::CredentialResolution { source } = error else {
        panic!("expected credential resolution error");
    };
    assert!(source.to_string().contains("expired"));

    twin.shutdown().await;
}

#[fabro_macros::e2e_test(twin)]
async fn branch_head_compatibility_wrapper_uses_repository_reader() {
    let twin = TwinGitHub::start(standard_app_state()).await;
    let credentials = github_credentials();
    let context = GitHubContext::with_http_client(
        &credentials,
        &twin.base_url,
        fabro_test::test_http_client(),
    );

    assert_eq!(
        branch_head_sha(&context, "acme", "widgets", "release")
            .await
            .unwrap(),
        Some(HEAD_SHA.to_string())
    );
    assert_eq!(
        branch_head_sha(&context, "acme", "widgets", "missing")
            .await
            .unwrap(),
        None
    );

    twin.shutdown().await;
}

#[fabro_macros::e2e_test(twin)]
async fn repository_reader_keeps_auth_and_malformed_sha_failures_distinct() {
    let mut state = standard_app_state();
    state.set_repository_ref("acme", "widgets", "heads/malformed", "short-sha");
    let denied_token =
        state.generate_access_token("42", 1, vec!["widgets".to_string()], serde_json::json!({}));
    let twin = TwinGitHub::start(state).await;

    let invalid_credentials = GitHubCredentials::Pat("invalid-token".to_string());
    let invalid_reader = GitHubRepositoryReader::open(
        &GitHubContext::with_http_client(
            &invalid_credentials,
            &twin.base_url,
            fabro_test::test_http_client(),
        ),
        &repository(),
    )
    .await
    .unwrap();
    assert!(matches!(
        invalid_reader.resolve_commit("heads/main").await,
        Err(RepositoryReadError::AuthenticationRejected)
    ));

    let denied_credentials = GitHubCredentials::Pat(denied_token);
    let denied_reader = GitHubRepositoryReader::open(
        &GitHubContext::with_http_client(
            &denied_credentials,
            &twin.base_url,
            fabro_test::test_http_client(),
        ),
        &repository(),
    )
    .await
    .unwrap();
    assert!(matches!(
        denied_reader.resolve_commit("heads/main").await,
        Err(RepositoryReadError::PermissionDenied)
    ));

    let app_credentials = github_credentials();
    let app_reader = GitHubRepositoryReader::open(
        &GitHubContext::with_http_client(
            &app_credentials,
            &twin.base_url,
            fabro_test::test_http_client(),
        ),
        &repository(),
    )
    .await
    .unwrap();
    assert!(matches!(
        app_reader.resolve_commit("heads/malformed").await,
        Err(RepositoryReadError::MalformedCommitSha)
    ));

    twin.shutdown().await;
}
