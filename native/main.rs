use cbgpu_deploy_mcp_native::{
    Failure, Result,
    api::{Api, DeploymentClient},
    environment, protocol,
};
use serde_json::json;
use std::{
    io::{IsTerminal, Write},
    sync::Arc,
};
fn main() {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => {
            eprintln!("MCP runtime unavailable.");
            std::process::exit(1)
        }
    };
    let result = runtime.block_on(run());
    // Tokio stdin may own an uncancellable OS read. Bound shutdown after cancelling async tasks,
    // so SIGTERM still drops refresh locks even when a client leaves its input pipe open.
    runtime.shutdown_timeout(std::time::Duration::from_secs(1));
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1)
    }
}
async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("serve");
    if ["help", "--help"].contains(&command) {
        eprintln!(
            "CBGPU Deploy MCP (native Rust)\n  cbgpu-deploy-mcp login    Sign in interactively\n  cbgpu-deploy-mcp status   Check authentication\n  cbgpu-deploy-mcp logout   Revoke only this login\n  cbgpu-deploy-mcp env      List/set/unset deployment environment variables\n  cbgpu-deploy-mcp          Run the stdio MCP server\n\nWorks with Codex, Claude Code and other stdio MCP clients."
        );
        return Ok(());
    }
    let api = Api::from_env()?;
    match command {
        "login" => {
            if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
                return Err(Failure::new(
                    "Run login in an interactive terminal. Never supply your password in a prompt, command argument or MCP configuration.",
                ));
            }
            if args.len() != 1 {
                return Err(Failure::new(
                    "Login does not accept credentials in command arguments.",
                ));
            }
            eprint!("CBGPU email: ");
            std::io::stderr()
                .flush()
                .map_err(|_| Failure::new("Terminal unavailable."))?;
            let mut email = String::new();
            std::io::stdin()
                .read_line(&mut email)
                .map_err(|_| Failure::new("Terminal unavailable."))?;
            let password = rpassword::prompt_password("CBGPU password (hidden): ")
                .map_err(|_| Failure::new("Private terminal input unavailable."))?;
            let account = api.login(email.trim(), &password).await?;
            eprintln!(
                "Connected as {}. Credentials saved privately at {}. Your password was not saved.",
                account.as_deref().unwrap_or("your CBGPU account"),
                api.file.display()
            );
        }
        "logout" => {
            api.logout().await?;
            eprintln!("CBGPU MCP session revoked and removed.");
        }
        "status" => {
            let data = api.request("/projects", "GET", None).await?;
            eprintln!(
                "Authenticated. Accessible frontend projects: {}",
                data["projects"].as_array().map_or(0, Vec::len)
            );
        }
        "env" => {
            if args.len() < 4 {
                return Err(usage());
            }
            let (action, kind, id) = (&args[1], &args[2], &args[3]);
            let result = match action.as_str() {
                "list" if args.len() == 4 => environment::list(&api, kind, id).await?,
                "set" if args.len() == 6 && args[4] == "--file" => {
                    let file = std::path::absolute(&args[5]).map_err(|_| usage())?;
                    environment::set(
                        &api,
                        kind,
                        id,
                        &environment::environment_file(file.to_str().ok_or_else(usage)?)?,
                    )
                    .await?
                }
                "unset" if args.len() == 6 && args[4] == "--keys" => {
                    let mut values = serde_json::Map::new();
                    for key in args[5].split(',').filter(|s| !s.is_empty()) {
                        values.insert(key.into(), json!(null));
                    }
                    environment::set(&api, kind, id, &serde_json::Value::Object(values)).await?
                }
                _ => return Err(usage()),
            };
            eprintln!(
                "{}",
                serde_json::to_string_pretty(&result).unwrap_or_default()
            );
        }
        "serve" => {
            tokio::select! {
                result=protocol::serve(Arc::new(api),tokio::io::stdin(),tokio::io::stdout())=>result.map_err(|_|Failure::new("MCP stdio connection closed."))?,
                _=termination_signal()=>{},
            }
        }
        _ => {
            return Err(Failure::new(
                "Unknown command. Run cbgpu-deploy-mcp --help.",
            ));
        }
    }
    Ok(())
}
fn usage() -> Failure {
    Failure::new(
        "Usage: cbgpu-deploy-mcp env list frontend|server PROJECT_ID; env set frontend|server PROJECT_ID --file .env.production; env unset frontend|server PROJECT_ID --keys KEY1,KEY2",
    )
}

async fn termination_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {_=term.recv()=>{},_=tokio::signal::ctrl_c()=>{}}
        } else {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
