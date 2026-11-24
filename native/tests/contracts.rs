use cbgpu_deploy_mcp_native::{
    Failure, Result,
    api::{self, Api, DeploymentClient, HttpRequest, Session, Transport},
    environment, tools,
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
const ID: &str = "550e8400-e29b-41d4-a716-446655440000";
#[derive(Default)]
struct MockApi {
    calls: Mutex<Vec<(String, String, Option<Value>)>>,
    viewer: bool,
    compose: bool,
}
#[async_trait::async_trait]
impl DeploymentClient for MockApi {
    async fn request(&self, p: &str, m: &str, b: Option<Value>) -> Result<Value> {
        self.calls.lock().unwrap().push((p.into(), m.into(), b));
        if p.starts_with("/compose/") {
            return Ok(
                json!({"status":"error","error":"rest unhealthy","logs":"database SYNTHETIC_PRIVATE_VALUE"}),
            );
        }
        Ok(
            json!({"project":{"id":ID,"collaboration_role":if self.viewer{"viewer"}else{"owner"},"env_vars":{"PASSWORD":"SYNTHETIC_PRIVATE_VALUE","KEEP":"retained","REMOVE":"delete-private","CBGPU_DEPLOY_ENGINE":if self.compose{"compose"}else{"git"}},"environment_keys":["OLD","REMOVE"]},"projects":[],"access_token":"NEVER_RETURN_ME"}),
        )
    }
}
fn text(v: &Value) -> &str {
    v["content"][0]["text"].as_str().unwrap()
}
#[test]
fn schema_has_all_seventeen_actual_tools() {
    let tools = tools::definitions();
    assert_eq!(tools.as_array().unwrap().len(), 19);
    for tool in tools.as_array().unwrap() {
        let name = tool["name"].as_str().unwrap();
        assert!(
            !["shell", "sql", "delete", "password"]
                .iter()
                .any(|word| name.contains(word))
        );
    }
}
#[tokio::test]
async fn real_schema_validation_and_output_sanitization() {
    let api = MockApi::default();
    let response = tools::call(&api, "list_projects", json!({"kind":"frontend"})).await;
    assert!(!text(&response).contains("NEVER_RETURN_ME"));
    assert!(!text(&response).contains("SYNTHETIC_PRIVATE_VALUE"));
    let bad=tools::call(&api,"create_frontend",json!({"slug":"valid-name","repository":"https://github.com/x/y","root_directory":"../private"})).await;
    assert_eq!(bad["isError"], true);
    assert_eq!(api.calls.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn server_settings_preserve_secrets_and_runtime() {
    let api = MockApi {
        compose: true,
        ..Default::default()
    };
    let result = tools::call(
        &api,
        "update_server",
        json!({"project_id":ID,"settings":{"root_directory":"server","memory_limit":"16g"}}),
    )
    .await;
    assert_ne!(result["isError"], true);
    let calls = api.calls.lock().unwrap();
    let payload = calls[1].2.as_ref().unwrap();
    assert_eq!(payload["env_vars"]["PASSWORD"], "SYNTHETIC_PRIVATE_VALUE");
    assert_eq!(payload["env_vars"]["CBGPU_DEPLOY_ENGINE"], "compose");
    assert_eq!(payload["env_vars"]["CBGPU_ROOT_DIRECTORY"], "server");
    assert_eq!(payload["memory_limit"], "16g");
    assert!(!text(&result).contains("SYNTHETIC_PRIVATE_VALUE"));
}
#[tokio::test]
async fn compose_creation_and_redeploy_follow_actual_async_api_contract() {
    let api = MockApi::default();
    let result=tools::call(&api,"create_server",json!({"name":"My backend","mode":"compose","settings":{"git_url":"https://github.com/example/app","root_directory":"server","container_port":8000,"gateway_service":"api-gw","cpu_limit":3,"memory_limit":"4g"}})).await;
    assert_ne!(result["isError"], true);
    {
        let calls = api.calls.lock().unwrap();
        let p = calls[0].2.as_ref().unwrap();
        assert_eq!(p["env_vars"]["CBGPU_DEPLOY_ENGINE"], "compose");
        assert_eq!(p["env_vars"]["CBGPU_ROOT_DIRECTORY"], "server");
        assert_eq!(p["env_vars"]["CBGPU_STORAGE_GB"], "40");
        assert_eq!(p["run_command"], "docker compose up -d");
    }
    tools::call(&api, "redeploy", json!({"kind":"server","project_id":ID})).await;
    let calls = api.calls.lock().unwrap();
    let timestamp = calls[1].2.as_ref().unwrap()["deploy_requested_at"]
        .as_str()
        .unwrap();
    assert!(chrono::DateTime::parse_from_rfc3339(timestamp).is_ok());
}
#[tokio::test]
async fn failed_health_is_not_relabelled_success_and_environment_secret_is_removed() {
    let api = MockApi {
        compose: true,
        ..Default::default()
    };
    let result = tools::call(
        &api,
        "deployment_logs",
        json!({"kind":"server","project_id":ID}),
    )
    .await;
    assert_ne!(result["isError"], true);
    assert!(text(&result).contains("rest unhealthy"));
    assert!(text(&result).contains("\"status\": \"error\""));
    assert!(!text(&result).contains("SYNTHETIC_PRIVATE_VALUE"));
}
#[tokio::test]
async fn viewers_cannot_modify_server_or_environment() {
    let api = MockApi {
        viewer: true,
        ..Default::default()
    };
    assert_eq!(
        tools::call(
            &api,
            "update_server",
            json!({"project_id":ID,"settings":{"name":"no"}})
        )
        .await["isError"],
        true
    );
    assert!(
        environment::set(&api, "server", ID, &json!({"SECRET":"no"}))
            .await
            .is_err()
    );
    assert!(
        api.calls
            .lock()
            .unwrap()
            .iter()
            .all(|(_, method, _)| method == "GET")
    );
}
#[test]
fn literal_dotenv_parsing_never_expands_shell_or_process_variables() {
    let p = environment::parse_dotenv(
        "TOKEN=\"secret # value\"\nMULTI=\"first\\nsecond\"\nEMPTY=\nLITERAL=${DO_NOT_READ}\nCOMMAND=$(do-not-execute)\n",
    );
    assert_eq!(p["TOKEN"], "secret # value");
    assert_eq!(p["MULTI"], "first\nsecond");
    assert_eq!(p["EMPTY"], "");
    assert_eq!(p["LITERAL"], "${DO_NOT_READ}");
    assert_eq!(p["COMMAND"], "$(do-not-execute)");
    assert!(environment::changes("server", &json!({"CBGPU_DEPLOY_ENGINE":"git"})).is_err());
    assert!(environment::changes("frontend", &json!({"SECRET":"a".repeat(8193)})).is_err());
}
#[test]
fn dotenv_file_constraints_reject_symlinks_oversize_and_wrong_names() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(".env.production");
    std::fs::write(&file, "TOKEN=private").unwrap();
    assert_eq!(
        environment::environment_file(file.to_str().unwrap()).unwrap()["TOKEN"],
        "private"
    );
    std::fs::write(&file, "a".repeat(131073)).unwrap();
    assert!(environment::environment_file(file.to_str().unwrap()).is_err());
    assert!(
        environment::environment_file(dir.path().join("credentials.json").to_str().unwrap())
            .is_err()
    );
    #[cfg(unix)]
    {
        let link = dir.path().join(".env.link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(environment::environment_file(link.to_str().unwrap()).is_err());
    }
}
#[tokio::test]
async fn environment_merges_preserve_system_settings_return_names_only_and_never_deploy() {
    let api = MockApi::default();
    let result = environment::set(
        &api,
        "server",
        ID,
        &json!({"TOKEN":"new-private","REMOVE":null}),
    )
    .await
    .unwrap();
    assert_eq!(result["updated_keys"], json!(["TOKEN"]));
    assert_eq!(result["removed_keys"], json!(["REMOVE"]));
    assert!(!result.to_string().contains("private"));
    let calls = api.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    let env = &calls[1].2.as_ref().unwrap()["env_vars"];
    assert_eq!(env["PASSWORD"], "SYNTHETIC_PRIVATE_VALUE");
    assert_eq!(env["CBGPU_DEPLOY_ENGINE"], "git");
    assert_eq!(env["TOKEN"], "new-private");
    assert!(env.get("REMOVE").is_none());
}
#[tokio::test]
async fn frontend_environment_only_sends_changes_and_list_omits_values() {
    let api = MockApi::default();
    environment::set(
        &api,
        "frontend",
        ID,
        &json!({"TOKEN":"new-private","REMOVE":null}),
    )
    .await
    .unwrap();
    assert_eq!(
        api.calls.lock().unwrap()[1].2,
        Some(json!({"env":{"TOKEN":"new-private","REMOVE":null}}))
    );
    let listed = environment::list(&api, "server", ID).await.unwrap();
    assert!(!listed.to_string().contains("SYNTHETIC_PRIVATE_VALUE"));
    assert!(
        !listed["environment_keys"]
            .as_array()
            .unwrap()
            .contains(&json!("CBGPU_DEPLOY_ENGINE"))
    );
}
struct MockTransport {
    requests: Mutex<Vec<HttpRequest>>,
    refresh: AtomicUsize,
    deny_old: bool,
    denied: Option<u16>,
    network: bool,
}
impl MockTransport {
    fn new() -> Self {
        Self {
            requests: Mutex::new(vec![]),
            refresh: AtomicUsize::new(0),
            deny_old: false,
            denied: None,
            network: false,
        }
    }
}
#[async_trait::async_trait]
impl Transport for MockTransport {
    async fn send(&self, r: HttpRequest) -> Result<Value> {
        let url = r.url.clone();
        let old = r
            .headers
            .get("Authorization")
            .is_some_and(|v| v == "Bearer old");
        self.requests.lock().unwrap().push(r);
        if self.network {
            return Err(Failure::new("Network unavailable."));
        }
        if url.contains("grant_type=password") {
            return Ok(
                json!({"access_token":"old","refresh_token":"initial-refresh","expires_at":1,"user":{"email":"fixture@example.invalid"}}),
            );
        }
        if url.contains("grant_type=refresh_token") {
            self.refresh.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            return Ok(
                json!({"access_token":"new","refresh_token":"rotated-refresh","expires_in":3600}),
            );
        }
        if let Some(status) = self.denied.or(if self.deny_old && old {
            Some(401)
        } else {
            None
        }) {
            return Err(Failure {
                status: Some(status),
                message: "Denied operation.".into(),
            });
        }
        Ok(json!({"projects":[]}))
    }
}
fn api(file: std::path::PathBuf, transport: Arc<MockTransport>) -> Api {
    Api::new("https://cbgpu.com", file, None, "fixture-anon".into())
        .unwrap()
        .with_transport(transport)
}
#[tokio::test]
async fn private_legacy_compatible_session_and_cross_instance_refresh_lock() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("auth/session.json");
    let mock = Arc::new(MockTransport::new());
    let a = api(file.clone(), mock.clone());
    a.login("fixture@example.invalid", "never-store-this")
        .await
        .unwrap();
    let raw = std::fs::read_to_string(&file).unwrap();
    assert!(!raw.contains("never-store-this"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let b = api(file, mock.clone());
    let (left, right) = tokio::join!(
        a.request("/projects", "GET", None),
        b.request("/servers/projects", "GET", None)
    );
    left.unwrap();
    right.unwrap();
    assert_eq!(mock.refresh.load(Ordering::SeqCst), 1);
    assert!(!dir.path().join("auth.lock").exists());
}
#[tokio::test]
async fn auth401_refreshes_once_but403_and_network_failures_never_retry_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("auth/session.json");
    let mut transport = MockTransport::new();
    transport.deny_old = true;
    let mock = Arc::new(transport);
    let a = api(file.clone(), mock.clone());
    a.prepare().unwrap();
    a.save(&Session {
        access_token: "old".into(),
        refresh_token: "refresh".into(),
        expires_at: cbgpu_deploy_mcp_native::epoch() + 3600.,
        base: a.base.clone(),
    })
    .unwrap();
    a.request(&format!("/projects/{ID}/deploy"), "POST", Some(json!({})))
        .await
        .unwrap();
    assert_eq!(mock.refresh.load(Ordering::SeqCst), 1);
    assert_eq!(mock.requests.lock().unwrap().len(), 3);
    for (network, denied) in [(true, None), (false, Some(403))] {
        let mut t = MockTransport::new();
        t.network = network;
        t.denied = denied;
        let t = Arc::new(t);
        let a = api(file.clone(), t.clone());
        assert!(
            a.request(&format!("/projects/{ID}/deploy"), "POST", Some(json!({})))
                .await
                .is_err()
        );
        assert_eq!(t.requests.lock().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn logout_revokes_only_local_session_before_deleting_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("auth/session.json");
    let mock = Arc::new(MockTransport::new());
    let a = api(file.clone(), mock.clone());
    a.prepare().unwrap();
    a.save(&Session {
        access_token: "old".into(),
        refresh_token: "refresh".into(),
        expires_at: 1.,
        base: a.base.clone(),
    })
    .unwrap();
    a.logout().await.unwrap();
    assert!(!file.exists());
    let requests = mock.requests.lock().unwrap();
    assert!(requests[1].url.ends_with("/logout?scope=local"));
}
#[test]
fn origins_permissions_and_redaction_preserve_credential_privacy() {
    for value in [
        "http://cbgpu.com",
        "https://user:password@cbgpu.com",
        "https://cbgpu.com/x",
        "https://cbgpu.com/?x=1",
    ] {
        assert!(api::base_url(value).is_err());
    }
    let result = api::sanitize(
        json!({"env_vars":{"FOO":"secret"},"access_token":"secret","repository":"https://name:secret@github.com/x/y","logs":"ghp_abcdef123"}),
    );
    assert!(!result.to_string().contains("secret"));
    assert_eq!(result["environment_keys"], json!(["FOO"]));
}
#[test]
fn repository_userinfo_is_redacted_even_when_an_opaque_token_is_the_username() {
    let value = api::sanitize(
        json!({"repository":"https://opaque-legacy-credential@github.com/example/private"}),
    );
    assert!(!value.to_string().contains("opaque-legacy-credential"));
    assert!(value.to_string().contains("[REDACTED]@github.com"));
}
#[test]
fn literal_dotenv_matches_existing_library_golden_files_including_export_and_crlf() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../protocol/dotenv-fixtures.json")).unwrap();
    for fixture in fixtures.as_array().unwrap() {
        assert_eq!(
            environment::parse_dotenv(fixture["input"].as_str().unwrap()),
            fixture["expected"]
        );
    }
}
