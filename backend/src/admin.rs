//! Password-protected store: page sessions, BMP instance registry and settings.
//!
//! Data is persisted to `<data-dir>/admin.json`. The page password is stored as an
//! Argon2id hash. Sessions live in memory and disappear on restart.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

pub const COOKIE_NAME: &str = "bmp_admin_session";
pub const SESSION_TTL_SECS: u64 = 12 * 60 * 60;
const PASSWORD_MIN_LEN: usize = 8;
const LOGIN_MAX_FAILURES: u32 = 5;
const LOGIN_LOCK_SECS: u64 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceRecord {
    pub id: String,
    pub name: String,
    /// Source IP of the BMP TCP connection (BIRD router address).
    pub router_ip: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub created_at: f64,
    #[serde(default)]
    pub updated_at: f64,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceInput {
    pub name: String,
    pub router_ip: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AdminFile {
    #[serde(default)]
    password_hash: Option<String>,
    #[serde(default)]
    enforce_allowlist: bool,
    #[serde(default)]
    instances: Vec<InstanceRecord>,
}

impl Default for AdminFile {
    fn default() -> Self {
        Self {
            password_hash: None,
            enforce_allowlist: false,
            instances: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Session {
    expires_at: u64,
}

#[derive(Debug, Clone, Copy, Default)]
struct LoginAttempt {
    failures: u32,
    locked_until: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginError {
    Invalid,
    Locked(u64),
}

/// Argon2 hashing is deliberately expensive, so it runs in `spawn_blocking` at the API layer.
pub struct AdminStore {
    file: PathBuf,
    data: Mutex<AdminFile>,
    sessions: Mutex<HashMap<String, Session>>,
    attempts: Mutex<HashMap<IpAddr, LoginAttempt>>,
    secure_cookies: bool,
}

impl AdminStore {
    pub fn load(
        data_dir: impl AsRef<Path>,
        admin_password: Option<&str>,
        secure_cookies: bool,
    ) -> anyhow::Result<Self> {
        let data_dir = data_dir.as_ref();
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("failed to create data dir {}", data_dir.display()))?;
        let file = data_dir.join("admin.json");

        let mut data = if file.exists() {
            let bytes = std::fs::read(&file)
                .with_context(|| format!("failed to read {}", file.display()))?;
            serde_json::from_slice::<AdminFile>(&bytes)
                .with_context(|| format!("failed to parse {}", file.display()))?
        } else {
            AdminFile::default()
        };

        let mut generated_password = None;
        let mut env_password = false;

        if let Some(pw) = admin_password {
            if pw.chars().count() < PASSWORD_MIN_LEN {
                bail!("ADMIN_PASSWORD must be at least {PASSWORD_MIN_LEN} characters");
            }
            data.password_hash = Some(hash_password(pw)?);
            env_password = true;
        } else if data.password_hash.is_none() {
            let pw = random_password(20);
            data.password_hash = Some(hash_password(&pw)?);
            generated_password = Some(pw);
        }

        let store = Self {
            file,
            data: Mutex::new(data),
            sessions: Mutex::new(HashMap::new()),
            attempts: Mutex::new(HashMap::new()),
            secure_cookies,
        };

        // Persist bootstrap changes (initial password or a reset via env).
        if generated_password.is_some() || env_password {
            let snapshot = store.snapshot();
            store.persist(&snapshot)?;
        }

        if let Some(pw) = generated_password {
            let path = data_dir.join("initial-admin-password.txt");
            let body = format!(
                "BMP Status initial page password\n\npassword: {pw}\n\n\
                 Please log in at / and change this password.\n"
            );
            if let Err(e) = write_private(&path, body.as_bytes()) {
                warn!(
                    "failed to write initial password file {}: {e}",
                    path.display()
                );
            }
            warn!("======================================================================");
            warn!("首次启动：页面密码已随机生成");
            warn!("  密码: {pw}");
            warn!("  已保存到: {}", path.display());
            warn!("  登录地址: / （登录后请立即修改密码）");
            warn!("======================================================================");
            if let Some(hash) = &store
                .data
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .password_hash
            {
                info!("page password hash stored (argon2id, {} chars)", hash.len());
            }
        } else if env_password {
            let stale = data_dir.join("initial-admin-password.txt");
            if stale.exists() {
                let _ = std::fs::remove_file(stale);
            }
            info!("page password reset from ADMIN_PASSWORD");
        } else {
            info!("page password loaded from {}", store.file.display());
        }

        Ok(store)
    }

    fn snapshot(&self) -> AdminFile {
        self.data.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn persist(&self, data: &AdminFile) -> anyhow::Result<()> {
        let parent = self
            .file
            .parent()
            .context("admin data file has no parent directory")?;
        std::fs::create_dir_all(parent)?;
        let tmp = self.file.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(data)?;
        write_private(&tmp, &bytes)?;
        std::fs::rename(&tmp, &self.file)
            .with_context(|| format!("failed to rename into {}", self.file.display()))?;
        Ok(())
    }

    pub fn password_configured(&self) -> bool {
        self.snapshot().password_hash.is_some()
    }

    pub fn login(&self, ip: IpAddr, password: &str) -> Result<(String, u64), LoginError> {
        let now = unix_now();
        {
            let mut attempts = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
            let attempt = attempts.entry(ip).or_default();
            if attempt.locked_until > now {
                return Err(LoginError::Locked(attempt.locked_until - now));
            }
            attempt.locked_until = 0;
        }

        let hash = self.snapshot().password_hash.ok_or(LoginError::Invalid)?;
        if verify_password(&hash, password) {
            self.attempts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&ip);
            Ok(self.create_session())
        } else {
            let mut attempts = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
            let attempt = attempts.entry(ip).or_default();
            attempt.failures += 1;
            if attempt.failures >= LOGIN_MAX_FAILURES {
                attempt.failures = 0;
                attempt.locked_until = now + LOGIN_LOCK_SECS;
                return Err(LoginError::Locked(LOGIN_LOCK_SECS));
            }
            Err(LoginError::Invalid)
        }
    }

    fn create_session(&self) -> (String, u64) {
        let token = random_hex(32);
        let expires_at = unix_now() + SESSION_TTL_SECS;
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.retain(|_, s| s.expires_at > unix_now());
        sessions.insert(token.clone(), Session { expires_at });
        (token, expires_at)
    }

    pub fn session_valid(&self, token: &str) -> Option<u64> {
        let now = unix_now();
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.retain(|_, s| s.expires_at > now);
        sessions.get(token).map(|s| s.expires_at)
    }

    pub fn logout(&self, token: &str) {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(token);
    }

    pub fn cookie_header(&self, token: &str, expires_at: u64) -> String {
        let max_age = expires_at.saturating_sub(unix_now());
        let secure = if self.secure_cookies { "; Secure" } else { "" };
        format!("{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}")
    }

    pub fn clear_cookie_header(&self) -> String {
        let secure = if self.secure_cookies { "; Secure" } else { "" };
        format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}")
    }

    pub fn change_password(&self, current: &str, new: &str) -> Result<(String, u64), String> {
        if new.chars().count() < PASSWORD_MIN_LEN {
            return Err(format!("新密码至少需要 {PASSWORD_MIN_LEN} 个字符"));
        }
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let hash = data
            .password_hash
            .as_ref()
            .ok_or_else(|| "页面密码尚未设置".to_string())?;
        if !verify_password(hash, current) {
            return Err("当前密码不正确".to_string());
        }
        let new_hash = hash_password(new).map_err(|e| e.to_string())?;
        let mut next = data.clone();
        next.password_hash = Some(new_hash);
        self.persist(&next).map_err(|e| e.to_string())?;
        *data = next;
        drop(data);

        // The bootstrap password file is no longer valid.
        if let Some(dir) = self.file.parent() {
            let stale = dir.join("initial-admin-password.txt");
            if stale.exists() {
                let _ = std::fs::remove_file(stale);
            }
        }

        // Invalidate all sessions, then issue a fresh one for the caller.
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        Ok(self.create_session())
    }

    pub fn instances(&self) -> Vec<InstanceRecord> {
        let mut list = self.snapshot().instances;
        list.sort_by(|a, b| {
            a.created_at
                .partial_cmp(&b.created_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        list
    }

    pub fn enforce_allowlist(&self) -> bool {
        self.snapshot().enforce_allowlist
    }

    pub fn is_source_allowed(&self, ip: IpAddr) -> bool {
        // Loopback is always trusted so local demos and local BIRD instances keep working.
        if ip.is_loopback() {
            return true;
        }
        let data = self.snapshot();
        if !data.enforce_allowlist {
            return true;
        }
        let canonical = ip.to_string();
        data.instances
            .iter()
            .any(|i| i.enabled && i.router_ip == canonical)
    }

    pub fn create_instance(&self, input: &InstanceInput) -> Result<InstanceRecord, String> {
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = data.clone();
        let (name, router_ip, description) = validate_instance(input)?;
        if next.instances.iter().any(|i| i.router_ip == router_ip) {
            return Err(format!("实例 IP {router_ip} 已存在"));
        }
        let now = unix_now_f64();
        let record = InstanceRecord {
            id: random_hex(6),
            name,
            router_ip,
            description,
            enabled: input.enabled,
            created_at: now,
            updated_at: now,
        };
        next.instances.push(record.clone());
        self.persist(&next).map_err(|e| e.to_string())?;
        *data = next;
        Ok(record)
    }

    pub fn update_instance(
        &self,
        id: &str,
        input: &InstanceInput,
    ) -> Result<InstanceRecord, String> {
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = data.clone();
        let (name, router_ip, description) = validate_instance(input)?;
        if next
            .instances
            .iter()
            .any(|i| i.id != id && i.router_ip == router_ip)
        {
            return Err(format!("实例 IP {router_ip} 已存在"));
        }
        let record = next
            .instances
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| "实例不存在".to_string())?;
        record.name = name;
        record.router_ip = router_ip;
        record.description = description;
        record.enabled = input.enabled;
        record.updated_at = unix_now_f64();
        let record = record.clone();
        self.persist(&next).map_err(|e| e.to_string())?;
        *data = next;
        Ok(record)
    }

    pub fn delete_instance(&self, id: &str) -> Result<(), String> {
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = data.clone();
        let before = next.instances.len();
        next.instances.retain(|i| i.id != id);
        if next.instances.len() == before {
            return Err("实例不存在".to_string());
        }
        self.persist(&next).map_err(|e| e.to_string())?;
        *data = next;
        Ok(())
    }

    pub fn set_enforce_allowlist(&self, enabled: bool) -> Result<(), String> {
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        let mut next = data.clone();
        next.enforce_allowlist = enabled;
        self.persist(&next).map_err(|e| e.to_string())?;
        *data = next;
        Ok(())
    }
}

fn validate_instance(input: &InstanceInput) -> Result<(String, String, String), String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("实例名称不能为空".to_string());
    }
    if name.chars().count() > 64 {
        return Err("实例名称不能超过 64 个字符".to_string());
    }
    let ip: IpAddr = input
        .router_ip
        .trim()
        .parse()
        .map_err(|_| format!("无效的 IP 地址: {}", input.router_ip.trim()))?;
    let description = input.description.trim();
    if description.chars().count() > 280 {
        return Err("描述不能超过 280 个字符".to_string());
    }
    Ok((name.to_string(), ip.to_string(), description.to_string()))
}

fn hash_password(password: &str) -> anyhow::Result<String> {
    let mut salt_bytes = [0u8; 16];
    getrandom::fill(&mut salt_bytes).context("OS random generator failed")?;
    let salt = SaltString::encode_b64(&salt_bytes)
        .map_err(|e| anyhow::anyhow!("failed to encode salt: {e}"))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| anyhow::anyhow!("failed to hash password: {e}"))
}

fn verify_password(hash: &str, password: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    if getrandom::fill(&mut buf).is_err() {
        // Extremely unlikely; fall back to time-based bytes rather than panicking.
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos()
            .to_le_bytes();
        for (i, b) in buf.iter_mut().enumerate() {
            *b = nanos[i % nanos.len()];
        }
    }
    let mut out = String::with_capacity(bytes * 2);
    for b in buf {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn random_password(len: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 32];
    while out.len() < len {
        if getrandom::fill(&mut buf).is_err() {
            break;
        }
        for b in buf {
            out.push(ALPHABET[(b as usize) % ALPHABET.len()] as char);
            if out.len() == len {
                break;
            }
        }
    }
    if out.len() < len {
        // Extremely unlikely fallback.
        while out.len() < len {
            out.push('x');
        }
    }
    out
}

fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?
    };
    #[cfg(not(unix))]
    let mut file = std::fs::File::create(path)?;

    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

pub fn unix_now_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs_f64()
}
