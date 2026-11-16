#[test]
fn native_cli_help_writes_no_protocol_stdout() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_cbgpu-deploy-mcp"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("CBGPU Deploy MCP"));
}
use cbgpu_deploy_mcp_native::{Failure, Result, api::DeploymentClient, protocol};
use serde_json::{Value, json};
use std::sync::Arc;
struct NoNetwork;
#[async_trait::async_trait]
impl DeploymentClient for NoNetwork {
    async fn request(&self, _: &str, _: &str, _: Option<Value>) -> Result<Value> {
        Err(Failure::new("No local login. Run cbgpu-deploy-mcp login."))
    }
}
#[tokio::test]
async fn protocol_negotiates_real_sdk_versions_and_tool_failures_remain_typed() {
    for version in [
        "2025-11-25",
        "2025-06-18",
        "2025-03-26",
        "2024-11-05",
        "2024-10-07",
    ] {
        let result=protocol::request(&NoNetwork,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}})).await.unwrap();
        assert_eq!(result["result"]["protocolVersion"], version);
    }
    let result=protocol::request(&NoNetwork,json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_projects","arguments":{"kind":"frontend"}}})).await.unwrap();
    assert_eq!(result["result"]["isError"], true);
    assert!(!result.to_string().contains("access_token"));
}
#[tokio::test]
async fn stdio_framing_recovers_from_bad_or_oversized_lines_without_logging_input() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut sender, input) = tokio::io::duplex(2 * 1024 * 1024);
    let (output, mut receiver) = tokio::io::duplex(65536);
    let server = tokio::spawn(protocol::serve(Arc::new(NoNetwork), input, output));
    let mut payload = b"invalid-json\n".to_vec();
    payload.extend(vec![b'x'; 1024 * 1024 + 1]);
    payload.extend(b"\n{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n");
    sender.write_all(&payload).await.unwrap();
    sender.shutdown().await.unwrap();
    let mut bytes = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        receiver.read_to_end(&mut bytes),
    )
    .await
    .unwrap()
    .unwrap();
    server.await.unwrap().unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let rows: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter().filter(|v| v["error"]["code"] == -32700).count(),
        2
    );
    assert!(rows.iter().any(|v| v["id"] == 7 && v["result"].is_object()));
    assert!(!text.contains("invalid-json"));
}
#[cfg(unix)]
#[test]
fn sigterm_cleans_only_its_own_active_refresh_lock() {
    use std::{
        io::Write,
        os::unix::fs::PermissionsExt,
        process::{Command, Stdio},
    };
    let dir = tempfile::tempdir().unwrap();
    let auth = dir.path().join("cbgpu-deploy-mcp");
    std::fs::create_dir(&auth).unwrap();
    std::fs::set_permissions(&auth, std::fs::Permissions::from_mode(0o700)).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("https://{}", listener.local_addr().unwrap());
    let file = auth.join("session.json");
    std::fs::write(&file,serde_json::to_vec(&json!({"base":base,"access_token":"expired-fixture","refresh_token":"private-fixture-refresh","expires_at":1})).unwrap()).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut child = Command::new(
        std::env::var_os("CBGPU_MCP_TEST_BINARY")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_cbgpu-deploy-mcp").into()),
    )
    .env("XDG_CONFIG_HOME", dir.path())
    .env("CBGPU_URL", &base)
    .env_remove("CBGPU_ACCESS_TOKEN")
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"list_projects\",\"arguments\":{\"kind\":\"frontend\"}}}\n").unwrap();
    input.flush().unwrap();
    let lock = dir.path().join("cbgpu-deploy-mcp.lock");
    let mut active = false;
    for _ in 0..300 {
        if lock.is_dir() {
            active = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if !active {
        let _ = child.kill();
        panic!("native refresh never acquired its lock");
    }
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    let mut exited = false;
    for _ in 0..300 {
        if child.try_wait().unwrap().is_some() {
            exited = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if !exited {
        let _ = child.kill();
    }
    assert!(exited);
    assert!(
        !lock.exists(),
        "graceful MCP termination left stale refresh lock"
    );
    assert!(
        file.exists(),
        "interrupting refresh must retain previous login"
    );
}
