use crate::{Failure, Result, epoch};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::{Duration, SystemTime},
};
pub const PUBLIC_ANON_KEY: &str = "sb_publishable_fWE68mJ0qDMA0CdgU2D5nK_7jWO2mVR";
static ENDPOINT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^/(projects|servers|compose|teams|domains)(/[-a-z0-9]+)*$").unwrap()
});
#[async_trait::async_trait]
pub trait DeploymentClient: Send + Sync {
    async fn request(&self, endpoint: &str, method: &str, body: Option<Value>) -> Result<Value>;
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: f64,
    pub base: String,
}
pub fn base_url(value: &str) -> Result<String> {
    let u = reqwest::Url::parse(value).map_err(|_| {
        Failure::new("CBGPU_URL must be an HTTPS origin without credentials or a path.")
    })?;
    if u.scheme() != "https"
        || !u.username().is_empty()
        || u.password().is_some()
        || u.path() != "/"
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err(Failure::new(
            "CBGPU_URL must be an HTTPS origin without credentials or a path.",
        ));
    }
    Ok(u.origin().ascii_serialization())
}
pub fn session_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| Failure::new("User home directory unavailable."))?;
    #[cfg(windows)]
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData/Local"));
    #[cfg(not(windows))]
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    Ok(root.join("cbgpu-deploy-mcp/session.json"))
}
pub struct HttpRequest {
    pub url: String,
    pub method: String,
    pub body: Option<Value>,
    pub headers: reqwest::header::HeaderMap,
}
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    async fn send(&self, request: HttpRequest) -> Result<Value>;
}
struct ReqwestTransport {
    client: reqwest::Client,
}
impl ReqwestTransport {
    fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| Failure::new("HTTP client unavailable."))?,
        })
    }
}
#[async_trait::async_trait]
impl Transport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<Value> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| Failure::new("Invalid request method."))?;
        let mut builder = self
            .client
            .request(method, request.url)
            .headers(request.headers);
        if let Some(body) = request.body {
            builder = builder.json(&body);
        }
        let mut response = builder.send().await.map_err(|_| {
            Failure::new("Unable to reach CBGPU. Check connectivity and service status.")
        })?;
        let status = response.status().as_u16();
        if status == 204 {
            return Ok(json!({}));
        }
        if !response.status().is_success() {
            return Err(Failure {
                status: Some(status),
                message: match status {
                    401 => "Login expired or account unavailable. Run cbgpu-deploy-mcp login.",
                    403 => "Your account does not have permission for this operation.",
                    429 => "CBGPU rate limit reached. Wait before retrying.",
                    _ => "CBGPU request failed. Check the website project settings.",
                }
                .into(),
            });
        }
        const LIMIT: usize = 2 * 1024 * 1024;
        if response.content_length().is_some_and(|n| n > LIMIT as u64) {
            return Err(Failure::new("CBGPU response exceeds the supported limit."));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| Failure::new("Unable to read CBGPU response."))?
        {
            if bytes.len() + chunk.len() > LIMIT {
                return Err(Failure::new("CBGPU response exceeds the supported limit."));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| {
            Failure::new("CBGPU returned a non-JSON response. Check service availability.")
        })
    }
}
#[derive(Clone)]
pub struct Api {
    pub base: String,
    pub file: PathBuf,
    pub token: Option<String>,
    pub anon: String,
    transport: Arc<dyn Transport>,
}
impl Api {
    pub fn new(base: &str, file: PathBuf, token: Option<String>, anon: String) -> Result<Self> {
        Ok(Self {
            base: base_url(base)?,
            file,
            token,
            anon,
            transport: Arc::new(ReqwestTransport::new()?),
        })
    }
    pub fn with_transport(mut self, transport: Arc<dyn Transport>) -> Self {
        self.transport = transport;
        self
    }
    pub fn from_env() -> Result<Self> {
        Self::new(
            &std::env::var("CBGPU_URL").unwrap_or_else(|_| "https://cbgpu.com".into()),
            session_path()?,
            std::env::var("CBGPU_ACCESS_TOKEN")
                .ok()
                .filter(|s| !s.is_empty()),
            std::env::var("CBGPU_ANON_KEY").unwrap_or_else(|_| PUBLIC_ANON_KEY.into()),
        )
    }
    async fn json(
        &self,
        endpoint: &str,
        method: &str,
        body: Option<Value>,
        token: Option<&str>,
        auth: bool,
    ) -> Result<Value> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Content-Type",
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            "Origin",
            self.base
                .parse()
                .map_err(|_| Failure::new("Invalid origin header."))?,
        );
        if let Some(token) = token {
            headers.insert(
                "Authorization",
                format!("Bearer {token}")
                    .parse()
                    .map_err(|_| Failure::new("Invalid access credential."))?,
            );
        }
        if auth {
            headers.insert(
                "apikey",
                self.anon
                    .parse()
                    .map_err(|_| Failure::new("Invalid publishable key."))?,
            );
        }
        self.transport
            .send(HttpRequest {
                url: format!("{}{endpoint}", self.base),
                method: method.into(),
                body,
                headers,
            })
            .await
    }
    pub fn prepare(&self) -> Result<()> {
        let parent = self
            .file
            .parent()
            .ok_or_else(|| Failure::new("Session directory unavailable."))?;
        std::fs::create_dir_all(parent)
            .map_err(|_| Failure::new("Cannot create private session directory."))?;
        validate_file(parent, true)?;
        secure(parent, true)
    }
    pub fn read(&self) -> Result<Session> {
        validate_file(&self.file, false)?;
        #[cfg(windows)]
        secure(&self.file, false)?;
        let bytes = std::fs::read(&self.file).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Failure::new("Run cbgpu-deploy-mcp login in your terminal first.")
            } else {
                Failure::new("Cannot read session file.")
            }
        })?;
        if bytes.len() > 65536 {
            return Err(Failure::new("Invalid session file."));
        }
        let session: Session = serde_json::from_slice(&bytes)
            .map_err(|_| Failure::new("Invalid session file. Log in again."))?;
        if session.base != self.base {
            return Err(Failure::new(
                "Session belongs to a different CBGPU origin. Log in again.",
            ));
        }
        Ok(session)
    }
    pub fn save(&self, session: &Session) -> Result<()> {
        use std::io::Write;
        let temp = self
            .file
            .with_extension(format!("json.{}", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&temp)
                .map_err(|_| Failure::new("Cannot save private session."))?;
            secure(&temp, false)?;
            serde_json::to_writer(
                &mut file,
                &Session {
                    access_token: session.access_token.clone(),
                    refresh_token: session.refresh_token.clone(),
                    expires_at: session.expires_at,
                    base: self.base.clone(),
                },
            )
            .map_err(|_| Failure::new("Cannot encode session."))?;
            file.flush()
                .map_err(|_| Failure::new("Cannot save session."))?;
            file.sync_all()
                .map_err(|_| Failure::new("Cannot save session."))?;
            drop(file);
            atomic_replace(&temp, &self.file)?;
            Ok(())
        })();
        let _ = std::fs::remove_file(temp);
        result
    }
    async fn locked(&self) -> Result<DirectoryLock> {
        self.prepare()?;
        DirectoryLock::acquire(self.file.parent().unwrap()).await
    }
    pub async fn login(&self, email: &str, password: &str) -> Result<Option<String>> {
        let _lock = self.locked().await?;
        let data = self
            .json(
                "/backend/auth/v1/token?grant_type=password",
                "POST",
                Some(json!({"email":email,"password":password})),
                None,
                true,
            )
            .await?;
        let session = self.session(&data)?;
        self.json(
            "/api/deploy/projects",
            "GET",
            None,
            Some(&session.access_token),
            false,
        )
        .await?;
        self.save(&session)?;
        Ok(data
            .pointer("/user/email")
            .and_then(Value::as_str)
            .map(String::from))
    }
    fn session(&self, data: &Value) -> Result<Session> {
        let access = data["access_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                Failure::new(
                    "No login session returned. Complete account verification on the website.",
                )
            })?;
        let refresh = data["refresh_token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Failure::new("Invalid refresh response. Log in again."))?;
        let expires = data["expires_at"]
            .as_f64()
            .filter(|n| *n > 0.)
            .unwrap_or_else(|| epoch() + data["expires_in"].as_f64().unwrap_or(0.));
        if !expires.is_finite() {
            return Err(Failure::new("Invalid session expiration."));
        }
        Ok(Session {
            access_token: access.into(),
            refresh_token: refresh.into(),
            expires_at: expires,
            base: self.base.clone(),
        })
    }
    async fn access(&self, rejected: Option<&str>) -> Result<String> {
        if let Some(token) = &self.token {
            return Ok(token.clone());
        }
        let _lock = self.locked().await?;
        let session = self.read()?;
        if session.expires_at > epoch() + 60.
            && rejected.is_none_or(|token| token != session.access_token)
        {
            return Ok(session.access_token);
        }
        let data = self
            .json(
                "/backend/auth/v1/token?grant_type=refresh_token",
                "POST",
                Some(json!({"refresh_token":session.refresh_token})),
                None,
                true,
            )
            .await?;
        let next = self.session(&data)?;
        self.save(&next)?;
        Ok(next.access_token)
    }
    pub async fn logout(&self) -> Result<()> {
        let _lock = self.locked().await?;
        if !self.file.exists() {
            return Ok(());
        }
        let session = self.read()?;
        let access = if session.expires_at > epoch() {
            session.access_token
        } else {
            let next = self
                .json(
                    "/backend/auth/v1/token?grant_type=refresh_token",
                    "POST",
                    Some(json!({"refresh_token":session.refresh_token})),
                    None,
                    true,
                )
                .await?;
            next["access_token"]
                .as_str()
                .ok_or_else(|| Failure::new("Invalid refresh response."))?
                .into()
        };
        self.json(
            "/backend/auth/v1/logout?scope=local",
            "POST",
            Some(json!({})),
            Some(&access),
            true,
        )
        .await?;
        std::fs::remove_file(&self.file)
            .map_err(|_| Failure::new("Cannot remove revoked session file."))?;
        Ok(())
    }
}
#[async_trait::async_trait]
impl DeploymentClient for Api {
    async fn request(&self, endpoint: &str, method: &str, body: Option<Value>) -> Result<Value> {
        if !ENDPOINT.is_match(endpoint) {
            return Err(Failure::new("Unsupported deployment endpoint."));
        }
        let token = self.access(None).await?;
        match self
            .json(
                &format!("/api/deploy{endpoint}"),
                method,
                body.clone(),
                Some(&token),
                false,
            )
            .await
        {
            Err(e) if e.status == Some(401) && self.token.is_none() => {
                let fresh = self.access(Some(&token)).await?;
                self.json(
                    &format!("/api/deploy{endpoint}"),
                    method,
                    body,
                    Some(&fresh),
                    false,
                )
                .await
            }
            result => result,
        }
    }
}
#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> Result<()> {
    std::fs::rename(source, destination).map_err(|_| Failure::new("Cannot commit private session."))
}
#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 1 | 8) } == 0 {
        return Err(Failure::new(
            "Cannot atomically replace private Windows session.",
        ));
    }
    Ok(())
}
fn validate_file(path: &Path, directory: bool) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Failure::new("Run cbgpu-deploy-mcp login in your terminal first.")
        } else {
            Failure::new("Cannot inspect session file.")
        }
    })?;
    if metadata.file_type().is_symlink()
        || (directory && !metadata.is_dir())
        || (!directory && !metadata.is_file())
    {
        return Err(Failure::new("Unsafe session path."));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::geteuid() }
            || (!directory && metadata.permissions().mode() & 0o077 != 0)
        {
            return Err(Failure::new("Session file must be private (chmod 600)."));
        }
    }
    Ok(())
}
fn secure(path: &Path, directory: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            path,
            std::fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
        )
        .map_err(|_| Failure::new("Cannot secure session storage."))?;
    }
    #[cfg(windows)]
    {
        let user = std::process::Command::new("whoami.exe")
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .map_err(|_| Failure::new("Windows identity unavailable."))?;
        let text = String::from_utf8_lossy(&user.stdout);
        let regex = Regex::new(r"S-\d-\d+(?:-\d+)+").unwrap();
        let sid = regex
            .find(&text)
            .ok_or_else(|| Failure::new("Windows identity unavailable."))?
            .as_str();
        let grant = format!("*{sid}:{}F", if directory { "(OI)(CI)" } else { "" });
        for args in [
            vec!["/reset"],
            vec!["/inheritance:r"],
            vec!["/grant:r", grant.as_str()],
        ] {
            let result = std::process::Command::new("icacls.exe")
                .arg(path)
                .args(args)
                .output()
                .map_err(|_| Failure::new("Cannot secure Windows session ACL."))?;
            if !result.status.success() {
                return Err(Failure::new("Cannot secure Windows session ACL."));
            }
        }
    }
    Ok(())
}
#[derive(Clone, PartialEq, Eq)]
struct LockIdentity {
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(not(unix))]
    created: SystemTime,
}
fn lock_identity(path: &Path) -> Option<LockIdentity> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(LockIdentity {
            dev: metadata.dev(),
            ino: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        Some(LockIdentity {
            created: metadata.created().ok()?,
        })
    }
}
struct DirectoryLock {
    path: PathBuf,
    identity: LockIdentity,
    heartbeat: tokio::task::JoinHandle<()>,
}
impl DirectoryLock {
    async fn acquire(directory: &Path) -> Result<Self> {
        let mut name = directory.as_os_str().to_os_string();
        name.push(".lock");
        let path = PathBuf::from(name);
        for attempt in 0..51 {
            match std::fs::create_dir(&path) {
                Ok(()) => {
                    if let Err(error) = secure(&path, true) {
                        let _ = std::fs::remove_dir(&path);
                        return Err(error);
                    }
                    let identity = lock_identity(&path)
                        .ok_or_else(|| Failure::new("Cannot identify session lock."))?;
                    let expected = identity.clone();
                    let copy = path.clone();
                    let heartbeat = tokio::spawn(async move {
                        loop {
                            tokio::time::sleep(Duration::from_secs(30)).await;
                            if lock_identity(&copy) != Some(expected.clone()) {
                                break;
                            }
                            let _ = filetime::set_file_mtime(
                                &copy,
                                filetime::FileTime::from_system_time(SystemTime::now()),
                            );
                        }
                    });
                    return Ok(Self {
                        path,
                        identity,
                        heartbeat,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::MetadataExt;
                            if metadata.uid() != unsafe { libc::geteuid() } {
                                return Err(Failure::new("Session lock belongs to another user."));
                            }
                        }
                        if metadata.file_type().is_symlink() || !metadata.is_dir() {
                            return Err(Failure::new("Unsafe session lock."));
                        }
                        if metadata
                            .modified()
                            .ok()
                            .and_then(|time| time.elapsed().ok())
                            .is_some_and(|age| age > Duration::from_secs(120))
                        {
                            let _ = std::fs::remove_dir(&path);
                            continue;
                        }
                    }
                }
                _ => return Err(Failure::new("Cannot acquire session lock.")),
            }
            if attempt == 50 {
                break;
            }
            tokio::time::sleep(Duration::from_millis((100 + attempt * 40).min(500))).await;
        }
        Err(Failure::new(
            "Session is busy. Retry after the other login/refresh completes.",
        ))
    }
}
impl Drop for DirectoryLock {
    fn drop(&mut self) {
        self.heartbeat.abort();
        if lock_identity(&self.path) == Some(self.identity.clone()) {
            let _ = std::fs::remove_dir(&self.path);
        }
    }
}

static CREDENTIAL_FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("(?i)token|password|secret|authorization|apikey").unwrap());
static URL_CREDENTIALS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(https?://)[^\s/@]+@").unwrap());
static JWT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b").unwrap());
static GITHUB_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:gh[pousr]_[A-Za-z0-9_]+|github_pat_[A-Za-z0-9_]+)\b").unwrap()
});
pub fn sanitize(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(sanitize).collect()),
        Value::Object(values) => {
            let mut out = serde_json::Map::new();
            for (key, item) in values {
                if ["env_vars", "env", "secrets"].contains(&key.as_str()) {
                    out.insert(
                        "environment_keys".into(),
                        Value::Array(
                            item.as_object()
                                .map(|o| o.keys().cloned().map(Value::String).collect())
                                .unwrap_or_default(),
                        ),
                    );
                } else if !CREDENTIAL_FIELD.is_match(&key) {
                    out.insert(key, sanitize(item));
                }
            }
            Value::Object(out)
        }
        Value::String(s) => {
            let credentials = URL_CREDENTIALS
                .replace_all(&s, "${1}[REDACTED]@")
                .into_owned();
            let jwt = JWT.replace_all(&credentials, "[REDACTED JWT]").into_owned();
            Value::String(
                GITHUB_TOKEN
                    .replace_all(&jwt, "[REDACTED GitHub token]")
                    .into_owned(),
            )
        }
        v => v,
    }
}
