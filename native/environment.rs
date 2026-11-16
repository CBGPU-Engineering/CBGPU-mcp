use crate::{Failure, Result, api::DeploymentClient};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};
pub fn changes(kind: &str, value: &Value) -> Result<Value> {
    let object = value
        .as_object()
        .filter(|o| !o.is_empty())
        .ok_or_else(|| Failure::new("Invalid environment changes."))?;
    let limit = match kind {
        "frontend" => 40,
        "server" => 160,
        _ => return Err(Failure::new("Invalid deployment kind.")),
    };
    if object.len() > limit {
        return Err(Failure::new(
            "Environment changes exceed the project variable limit.",
        ));
    }
    for (key, value) in object {
        let mut chars = key.chars();
        if key.len() > 100
            || !chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            || key.starts_with("CBGPU_")
            || ["__proto__", "prototype", "constructor"].contains(&key.as_str())
        {
            return Err(Failure::new(
                "Invalid or reserved environment variable name. CBGPU_ settings use the project settings tools.",
            ));
        }
        if !value.is_null()
            && !value
                .as_str()
                .is_some_and(|s| s.encode_utf16().count() <= 8192 && !s.contains('\0'))
        {
            return Err(Failure::new(
                "Environment values must be strings up to 8192 characters, or null to remove a variable.",
            ));
        }
    }
    if serde_json::to_vec(value).unwrap_or_default().len() > 60000 {
        return Err(Failure::new(
            "Environment payload exceeds the API request limit.",
        ));
    }
    Ok(value.clone())
}
pub fn parse_dotenv(text: &str) -> Value {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = normalized.trim_start_matches('\u{feff}');
    let mut result = serde_json::Map::new();
    let lines: Vec<&str> = text.split('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        let mut line = lines[i].trim_start();
        i += 1;
        if let Some(rest) = line
            .strip_prefix("export")
            .filter(|rest| rest.chars().next().is_some_and(char::is_whitespace))
        {
            line = rest.trim_start();
        }
        let colon = line.find(':').filter(|index| {
            line[index + 1..]
                .chars()
                .next()
                .is_some_and(char::is_whitespace)
        });
        let index = match (line.find('='), colon) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let Some(index) = index else {
            continue;
        };
        let key = line[..index].trim();
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        {
            continue;
        }
        let raw = line[index + 1..].trim_start();
        let value = if let Some(quote) = raw.chars().next().filter(|c| ['\'', '"', '`'].contains(c))
        {
            let mut joined = raw[quote.len_utf8()..].to_owned();
            let mut end = closing_quote(&joined, quote);
            while end.is_none() && i < lines.len() {
                joined.push('\n');
                joined.push_str(lines[i]);
                i += 1;
                end = closing_quote(&joined, quote);
            }
            let mut value = joined[..end.unwrap_or(joined.len())].to_owned();
            if quote == '"' {
                value = value.replace("\\n", "\n").replace("\\r", "\r");
            }
            value
        } else {
            raw.split('#').next().unwrap_or_default().trim().to_owned()
        };
        result.insert(key.into(), Value::String(value));
    }
    Value::Object(result)
}
fn closing_quote(value: &str, quote: char) -> Option<usize> {
    let mut escape = false;
    for (index, c) in value.char_indices() {
        if c == quote && !escape {
            return Some(index);
        }
        escape = c == '\\' && !escape;
    }
    None
}
pub fn environment_file(filename: &str) -> Result<Value> {
    let path = Path::new(filename);
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if !path.is_absolute()
        || !(name == ".env" || name.starts_with(".env.") || name.ends_with(".env"))
    {
        return Err(Failure::new(
            "Provide an absolute path to an explicitly chosen .env file.",
        ));
    }
    let read = || -> Result<Value> {
        use std::io::Read;
        let stat = std::fs::symlink_metadata(path)
            .map_err(|_| Failure::new("Cannot inspect environment file."))?;
        if stat.file_type().is_symlink() || !stat.is_file() || stat.len() > 131072 {
            return Err(Failure::new("Invalid environment file."));
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x00200000);
        }
        let file = options
            .open(path)
            .map_err(|_| Failure::new("Cannot read environment file."))?;
        if !file
            .metadata()
            .map_err(|_| Failure::new("Cannot inspect environment file."))?
            .is_file()
        {
            return Err(Failure::new("Invalid environment file."));
        }
        let mut bytes = Vec::new();
        file.take(131073)
            .read_to_end(&mut bytes)
            .map_err(|_| Failure::new("Cannot read environment file."))?;
        if bytes.len() > 131072 {
            return Err(Failure::new("Environment file too large."));
        }
        let text =
            String::from_utf8(bytes).map_err(|_| Failure::new("Invalid environment encoding."))?;
        let parsed = parse_dotenv(&text);
        if parsed.as_object().is_none_or(|o| o.is_empty()) {
            return Err(Failure::new("Empty environment file."));
        }
        Ok(parsed)
    };
    read().map_err(|_|Failure::new("Cannot read the selected .env file. It must be a regular non-symlink file, at most 128 KiB, containing environment assignments."))
}
pub fn project_path(kind: &str, id: &str) -> Result<String> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(Failure::new("Invalid project ID."));
    }
    match kind {
        "frontend" => Ok(format!("/projects/{id}")),
        "server" => Ok(format!("/servers/projects/{id}")),
        _ => Err(Failure::new("Invalid deployment kind.")),
    }
}
pub async fn list(api: &dyn DeploymentClient, kind: &str, id: &str) -> Result<Value> {
    let data = api.request(&project_path(kind, id)?, "GET", None).await?;
    let project = &data["project"];
    let mut keys: BTreeSet<String> = project["environment_keys"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect();
    if let Some(env) = project["env_vars"].as_object() {
        keys.extend(env.keys().cloned());
    }
    keys.retain(|key| !key.starts_with("CBGPU_"));
    Ok(json!({"project_id":id,"kind":kind,"environment_keys":keys}))
}
pub async fn set(api: &dyn DeploymentClient, kind: &str, id: &str, input: &Value) -> Result<Value> {
    let path = project_path(kind, id)?;
    let values = changes(kind, input)?;
    let data = api.request(&path, "GET", None).await?;
    let project = &data["project"];
    if ![Some("owner"), Some("editor")].contains(&project["collaboration_role"].as_str()) {
        return Err(Failure::new(
            "Editing environment variables requires project owner or editor access.",
        ));
    }
    let object = values.as_object().unwrap();
    let payload = if kind == "frontend" {
        let mut keys: BTreeSet<String> = project["environment_keys"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect();
        for (k, v) in object {
            if v.is_null() {
                keys.remove(k);
            } else {
                keys.insert(k.clone());
            }
        }
        if keys.len() > 40 {
            return Err(Failure::new(
                "Frontend environment variable limit exceeded.",
            ));
        }
        json!({"env":values})
    } else {
        let mut env = project["env_vars"].as_object().cloned().unwrap_or_default();
        for (k, v) in object {
            if v.is_null() {
                env.remove(k);
            } else {
                env.insert(k.clone(), v.clone());
            }
        }
        if env.len() > 160 {
            return Err(Failure::new("Server environment variable limit exceeded."));
        }
        json!({"env_vars":env})
    };
    if serde_json::to_vec(&payload).unwrap_or_default().len() > 64000 {
        return Err(Failure::new(
            "Merged environment exceeds the API request limit.",
        ));
    }
    api.request(&path, "PATCH", Some(payload)).await?;
    let updated: BTreeSet<_> = object
        .iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, _)| k)
        .collect();
    let removed: BTreeSet<_> = object
        .iter()
        .filter(|(_, v)| v.is_null())
        .map(|(k, _)| k)
        .collect();
    Ok(
        json!({"ok":true,"project_id":id,"kind":kind,"updated_keys":updated,"removed_keys":removed,"redeploy_required":true}),
    )
}
