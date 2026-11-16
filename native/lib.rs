pub mod api;
pub mod environment;
pub mod protocol;
pub mod tools;
#[derive(Debug, Clone)]
pub struct Failure {
    pub message: String,
    pub status: Option<u16>,
}
impl Failure {
    pub fn new(message: &str) -> Self {
        Self {
            message: message.into(),
            status: None,
        }
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Failure {}
pub type Result<T> = std::result::Result<T, Failure>;
pub fn epoch() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}
