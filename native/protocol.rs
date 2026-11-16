use crate::{api::DeploymentClient, tools};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
pub async fn request(api: &dyn DeploymentClient, message: Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    if !(id.is_string() || id.is_number()) {
        return Some(error(Value::Null, -32600, "Invalid request ID."));
    }
    if message["jsonrpc"] != "2.0" {
        return Some(error(id, -32600, "Invalid JSON-RPC request."));
    }
    let result = match message["method"].as_str() {
        Some("initialize") => {
            let requested = message
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let version = if [
                "2025-11-25",
                "2025-06-18",
                "2025-03-26",
                "2024-11-05",
                "2024-10-07",
            ]
            .contains(&requested)
            {
                requested
            } else {
                "2025-11-25"
            };
            json!({"protocolVersion":version,"capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"cbgpu-deploy","version":env!("CARGO_PKG_VERSION")},"instructions":tools::INSTRUCTIONS})
        }
        Some("ping") => json!({}),
        Some("tools/list") => json!({"tools":tools::definitions()}),
        Some("tools/call") => {
            let Some(name) = message.pointer("/params/name").and_then(Value::as_str) else {
                return Some(error(id, -32602, "Tool name required."));
            };
            tools::call(
                api,
                name,
                message
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            )
            .await
        }
        Some("resources/list") => json!({"resources":[]}),
        Some("prompts/list") => json!({"prompts":[]}),
        _ => return Some(error(id, -32601, "Method not found.")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}
pub async fn serve<R, W>(
    api: Arc<dyn DeploymentClient>,
    mut input: R,
    mut output: W,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Value>(64);
    let writer = tokio::spawn(async move {
        while let Some(value) = rx.recv().await {
            let mut bytes = serde_json::to_vec(&value).map_err(std::io::Error::other)?;
            bytes.push(b'\n');
            output.write_all(&bytes).await?;
            output.flush().await?;
        }
        Ok::<(), std::io::Error>(())
    });
    let permits = Arc::new(tokio::sync::Semaphore::new(16));
    let mut jobs = tokio::task::JoinSet::new();
    let mut cancellations: HashMap<String, tokio::task::AbortHandle> = HashMap::new();
    let mut buffer = [0; 8192];
    let mut frame = Vec::new();
    let mut discarding = false;
    loop {
        let count = input.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        for byte in &buffer[..count] {
            if *byte != b'\n' {
                if !discarding {
                    frame.push(*byte);
                    if frame.len() > 1024 * 1024 {
                        frame.clear();
                        discarding = true;
                    }
                }
                continue;
            }
            if discarding {
                discarding = false;
                let _ = tx
                    .send(error(Value::Null, -32700, "MCP frame exceeds 1 MiB."))
                    .await;
                continue;
            }
            if frame.iter().all(u8::is_ascii_whitespace) {
                frame.clear();
                continue;
            }
            let parsed = serde_json::from_slice::<Value>(&frame);
            frame.clear();
            let message = match parsed {
                Ok(v) => v,
                Err(_) => {
                    let _ = tx.send(error(Value::Null, -32700, "Invalid JSON.")).await;
                    continue;
                }
            };
            if message["method"] == "notifications/cancelled" {
                if let Some(id) = message.pointer("/params/requestId")
                    && let Some(handle) = cancellations.remove(&id.to_string())
                {
                    handle.abort();
                }
                continue;
            }
            if message.get("id").is_none() {
                continue;
            }
            let id = message["id"].clone();
            let Ok(permit) = permits.clone().try_acquire_owned() else {
                let _ = tx
                    .send(error(id, -32000, "Too many concurrent MCP requests."))
                    .await;
                continue;
            };
            let key = id.to_string();
            if cancellations.contains_key(&key) {
                let _ = tx
                    .send(error(id, -32600, "Request ID is already active."))
                    .await;
                continue;
            }
            let api = api.clone();
            let sender = tx.clone();
            let handle = jobs.spawn(async move {
                let _permit = permit;
                if let Some(response) = request(api.as_ref(), message).await {
                    let _ = sender.send(response).await;
                }
                key
            });
            cancellations.insert(id.to_string(), handle);
            while let Some(result) = jobs.try_join_next() {
                if let Ok(key) = result {
                    cancellations.remove(&key);
                }
            }
        }
    }
    // EOF drains admitted operations rather than dropping a possibly accepted deployment mutation.
    while let Some(result) = jobs.join_next().await {
        if let Ok(key) = result {
            cancellations.remove(&key);
        }
    }
    drop(tx);
    writer.await.map_err(std::io::Error::other)?
}
