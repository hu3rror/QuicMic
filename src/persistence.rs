//! Machine-local persisted identity: TLS keypair + pairing PIN reused across
//! restarts (ADR-0015).
//!
//! The server stores two independent artifacts in the data directory:
//! `identity.json` (certificate keypair + the LAN IP it was built for + creation
//! time) and a separate plaintext `pin` file. Rotation or corruption of one never
//! touches the other.

use std::fs;
use std::io;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use std::io::Write;
use tracing::warn;

use crate::tls;

/// Certificate rotation threshold: regenerate once the certificate is this old,
/// keeping it inside the W3C two-week ceiling for `serverCertificateHashes`.
pub const ROTATION_AGE_SECS: u64 = 13 * 24 * 60 * 60;

const IDENTITY_FILE: &str = "identity.json";
const PIN_FILE: &str = "pin";

/// Where the persisted identity lives.
pub enum DataDir {
    /// Explicitly configured via `--data-dir` or `QUICMIC_DATA_DIR`: an unusable
    /// location is a hard startup error.
    Explicit(PathBuf),
    /// Platform default: an unusable location degrades to an ephemeral identity.
    Default(PathBuf),
    /// No data directory available at all: ephemeral identity for this run.
    None,
}

/// Resolve the data directory. Precedence: CLI flag > env var > platform default.
pub fn resolve_data_dir(
    cli: Option<&Path>,
    env_data_dir: Option<&Path>,
    platform_default: Option<&Path>,
) -> DataDir {
    if let Some(p) = cli {
        DataDir::Explicit(p.to_path_buf())
    } else if let Some(p) = env_data_dir {
        DataDir::Explicit(p.to_path_buf())
    } else if let Some(p) = platform_default {
        DataDir::Default(p.to_path_buf())
    } else {
        DataDir::None
    }
}

/// Platform default data directory (reads the environment).
pub fn platform_data_dir() -> Option<PathBuf> {
    resolve_platform_default(
        env_path("LOCALAPPDATA").as_deref(),
        env_path("APPDATA").as_deref(),
        env_path("XDG_DATA_HOME").as_deref(),
        env_path("HOME")
            .or_else(|| env_path("USERPROFILE"))
            .as_deref(),
    )
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(PathBuf::from)
}

/// Pure platform-default resolution, injectable for tests:
/// - Windows: `%LOCALAPPDATA%\QuicMic`, falling back to `%APPDATA%`, then the home dir.
/// - macOS: `~/Library/Application Support/QuicMic`.
/// - Linux/other: `$XDG_DATA_HOME/QuicMic`, else `~/.local/share/QuicMic`.
fn resolve_platform_default(
    local_appdata: Option<&Path>,
    appdata: Option<&Path>,
    xdg_data_home: Option<&Path>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let _ = xdg_data_home;
        local_appdata
            .or(appdata)
            .or(home)
            .map(|base| base.join("QuicMic"))
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (local_appdata, appdata, xdg_data_home);
        home.map(|h| {
            h.join("Library")
                .join("Application Support")
                .join("QuicMic")
        })
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (local_appdata, appdata);
        xdg_data_home
            .map(|p| p.join("QuicMic"))
            .or_else(|| home.map(|h| h.join(".local").join("share").join("QuicMic")))
    }
}

/// A resolved data directory after `open_store`: either a working persisting
/// store, or nothing to persist to.
pub enum StoreResult {
    /// Persisting store; failures from here on are fatal (explicit `--data-dir` /
    /// `QUICMIC_DATA_DIR`).
    Persist(IdentityStore),
    /// No usable data directory: the server still starts with a one-shot
    /// in-memory identity and PIN; later store failures downgrade to ephemeral
    /// material instead of aborting startup (ADR-0015).
    Ephemeral,
}

/// Map a resolved data dir to a store: an explicit location is a hard error, a
/// platform-default location degrades to an ephemeral identity (ADR-0015).
pub fn open_store(dir: DataDir) -> anyhow::Result<StoreResult> {
    match dir {
        DataDir::Explicit(path) => {
            let store = IdentityStore::new(path);
            store
                .prepare()
                .map_err(|e| anyhow::anyhow!("data directory unusable: {e:#}"))?;
            Ok(StoreResult::Persist(store))
        }
        DataDir::Default(path) => {
            let store = IdentityStore::new(path);
            match store.prepare() {
                Ok(()) => Ok(StoreResult::Persist(store)),
                Err(e) => {
                    warn!(
                        dir = ?store.path(),
                        error = %e,
                        "Data directory unusable; falling back to an ephemeral identity (nothing will persist)"
                    );
                    Ok(StoreResult::Ephemeral)
                }
            }
        }
        DataDir::None => {
            warn!(
                "No data directory available; using an ephemeral identity (nothing will persist)"
            );
            Ok(StoreResult::Ephemeral)
        }
    }
}

/// The `--pin` argument, reduced to an action.
pub enum PinArg {
    /// `--pin <6 digits>`: validate, use, and persist (write-through).
    Fixed(String),
    /// `--pin random`: generate and persist a fresh PIN.
    Random,
    /// Omitted: reuse the stored PIN, or generate one on first run.
    Default,
}

/// Parse and validate the `--pin` argument. Kept here so the 6-digit rule and the
/// `random` sentinel live next to the PIN store they feed.
pub fn parse_pin_arg(arg: Option<&str>) -> anyhow::Result<PinArg> {
    match arg {
        None => Ok(PinArg::Default),
        Some("random") => Ok(PinArg::Random),
        Some(pin) if is_valid_pin(pin) => Ok(PinArg::Fixed(pin.to_string())),
        Some(other) => {
            anyhow::bail!("--pin must be exactly 6 digits (0-9) or \"random\", got: {other}")
        }
    }
}

pub fn random_pin() -> String {
    format!("{:06}", rand::random_range(0..1_000_000u32))
}

fn is_valid_pin(pin: &str) -> bool {
    pin.len() == 6 && pin.bytes().all(|b| b.is_ascii_digit())
}

/// Seconds since the Unix epoch (used for rotation age decisions).
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Serialize, Deserialize)]
struct IdentityFile {
    cert_pem: String,
    key_pem: String,
    lan_ip: String,
    created_at: u64,
}

/// The persisted-identity store for one data directory.
pub struct IdentityStore {
    dir: PathBuf,
}

impl IdentityStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Create the data directory and verify it is writable. The caller maps an
    /// error to a hard startup failure (explicit dir) or an ephemeral fallback
    /// (platform default).
    pub fn prepare(&self) -> anyhow::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let probe = self.dir.join(".write-probe");
        fs::write(&probe, b"")?;
        fs::remove_file(&probe)?;
        Ok(())
    }

    /// Load the persisted identity for `lan_ip`, or generate and persist one.
    ///
    /// Regenerates (cert only, PIN untouched) when the stored `lan_ip` differs,
    /// the certificate is at rotation age, or the stored file is corrupt/unusable
    /// (quarantined aside as `<file>.corrupt-<ts>` first).
    pub fn load_or_create(
        &self,
        lan_ip: IpAddr,
        now: u64,
    ) -> anyhow::Result<(wtransport::Identity, tls::TlsIdentity)> {
        let path = self.dir.join(IDENTITY_FILE);
        match self.try_load(&path, lan_ip, now) {
            Ok(Some(identity)) => return Ok(identity),
            Ok(None) => {}
            Err(e) => {
                warn!(
                    file = ?path,
                    error = %e,
                    "Persisted identity corrupt or unusable; quarantining and regenerating"
                );
                self.quarantine(&path)?;
            }
        }
        let (wt_identity, identity) = tls::generate_identity(lan_ip)?;
        let file = IdentityFile {
            cert_pem: identity.cert_pem.clone(),
            key_pem: identity.key_pem.clone(),
            lan_ip: lan_ip.to_string(),
            created_at: now,
        };
        self.atomic_write(&path, &serde_json::to_vec(&file)?)?;
        Ok((wt_identity, identity))
    }

    /// Load the persisted PIN, generating and persisting one on first run or when
    /// the stored value is invalid/missing (quarantined aside first).
    pub fn read_pin(&self) -> anyhow::Result<String> {
        let path = self.dir.join(PIN_FILE);
        match fs::read_to_string(&path) {
            Ok(text) => {
                let pin = text.trim().to_string();
                if is_valid_pin(&pin) {
                    return Ok(pin);
                }
                warn!(file = ?path, "Stored PIN invalid; quarantining and regenerating");
                self.quarantine(&path)?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                warn!(file = ?path, error = %e, "Stored PIN unreadable; quarantining and regenerating");
                self.quarantine(&path)?;
            }
        }
        let pin = random_pin();
        self.write_pin(&pin)?;
        Ok(pin)
    }

    /// Validate and persist a PIN (write-through for `--pin <digits>`).
    pub fn write_pin(&self, pin: &str) -> anyhow::Result<()> {
        if !is_valid_pin(pin) {
            anyhow::bail!("PIN must be exactly 6 digits (0-9)");
        }
        self.atomic_write(&self.dir.join(PIN_FILE), pin.as_bytes())?;
        Ok(())
    }

    /// Generate, persist, and return a fresh random PIN.
    pub fn write_random_pin(&self) -> anyhow::Result<String> {
        let pin = random_pin();
        self.write_pin(&pin)?;
        Ok(pin)
    }

    /// Try to load a *current* identity (matching IP, fresh, valid). `None` means
    /// "fail over to generation" (missing file, IP change, or rotation age);
    /// `Err` means the file is corrupt and should be quarantined.
    fn try_load(
        &self,
        path: &Path,
        lan_ip: IpAddr,
        now: u64,
    ) -> anyhow::Result<Option<(wtransport::Identity, tls::TlsIdentity)>> {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let file: IdentityFile = serde_json::from_str(&text)?;
        if file.lan_ip != lan_ip.to_string() {
            return Ok(None);
        }
        if now.saturating_sub(file.created_at) >= ROTATION_AGE_SECS {
            return Ok(None);
        }
        Ok(Some(tls::from_pem(&file.cert_pem, &file.key_pem)?))
    }

    /// Rename a corrupt file aside as `<name>.corrupt-<unix>-<pid>` so the
    /// evidence survives regeneration instead of being silently overwritten, and
    /// two starting instances can't collide on the quarantine name.
    fn quarantine(&self, path: &Path) -> io::Result<PathBuf> {
        let mut name = path
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
            .to_os_string();
        name.push(format!(".corrupt-{}-{}", now_secs(), std::process::id()));
        let dst = path.with_file_name(name);
        fs::rename(path, &dst)?;
        Ok(dst)
    }

    /// Write `path` atomically: write a sibling temp file (same dir → same
    /// filesystem), fsync, then rename over the target. On Unix the file is
    /// created mode `0600`; on Windows the caller's profile ACLs apply. The temp
    /// name embeds the pid so two starting instances never share a temp file.
    fn atomic_write(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        let tmp = path.with_file_name(format!(
            "{}.{}.tmp",
            path.file_name()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
                .to_string_lossy(),
            std::process::id()
        ));
        let result = (|| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                let mut f = fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&tmp)?;
                f.write_all(contents)?;
                f.sync_all()?;
            }
            #[cfg(not(unix))]
            {
                let mut f = fs::File::create(&tmp)?;
                f.write_all(contents)?;
                f.sync_all()?;
            }
            fs::rename(&tmp, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Unique empty temp directory per test, without adding a dev-dependency.
    fn temp_dir(tag: &str) -> PathBuf {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "quicmic-persistence-test-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn test_ip() -> IpAddr {
        "192.168.1.42".parse().unwrap()
    }

    /// Ensure the ring provider is installed (main.rs does this in `run`; tests
    /// must too, because `tls::from_pem` builds a rustls ServerConfig).
    fn ensure_ring_provider() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    // ── data-dir resolution ────────────────────────────────────────────

    #[test]
    fn resolve_prefers_cli_over_env_over_default() {
        let cli = Path::new("/cli");
        let env = Path::new("/env");
        let def = Path::new("/default");
        match resolve_data_dir(Some(cli), Some(env), Some(def)) {
            DataDir::Explicit(p) => assert_eq!(p, PathBuf::from("/cli")),
            _ => panic!("CLI flag must win"),
        }
        match resolve_data_dir(None, Some(env), Some(def)) {
            DataDir::Explicit(p) => assert_eq!(p, PathBuf::from("/env")),
            _ => panic!("env must be explicit and beat the default"),
        }
        match resolve_data_dir(None, None, Some(def)) {
            DataDir::Default(p) => assert_eq!(p, PathBuf::from("/default")),
            _ => panic!("platform default used when nothing configured"),
        }
        assert!(matches!(resolve_data_dir(None, None, None), DataDir::None));
    }

    #[test]
    fn platform_default_prefers_local_appdata_on_windows() {
        #[cfg(windows)]
        {
            let got = resolve_platform_default(
                Some(Path::new(r"C:\Users\me\AppData\Local")),
                Some(Path::new(r"C:\Users\me\AppData\Roaming")),
                None,
                Some(Path::new(r"C:\Users\me")),
            );
            assert_eq!(
                got,
                Some(PathBuf::from(r"C:\Users\me\AppData\Local\QuicMic"))
            );
            // Fallback chain: LOCALAPPDATA missing → APPDATA → home.
            let fallback = resolve_platform_default(
                None,
                Some(Path::new(r"C:\Users\me\AppData\Roaming")),
                None,
                Some(Path::new(r"C:\Users\me")),
            );
            assert_eq!(
                fallback,
                Some(PathBuf::from(r"C:\Users\me\AppData\Roaming\QuicMic"))
            );
            let home_fallback =
                resolve_platform_default(None, None, None, Some(Path::new(r"C:\Users\me")));
            assert_eq!(home_fallback, Some(PathBuf::from(r"C:\Users\me\QuicMic")));
        }
        #[cfg(target_os = "macos")]
        {
            let got = resolve_platform_default(None, None, None, Some(Path::new("/Users/me")));
            assert_eq!(
                got,
                Some(PathBuf::from(
                    "/Users/me/Library/Application Support/QuicMic"
                ))
            );
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let got = resolve_platform_default(
                None,
                None,
                Some(Path::new("/xdg")),
                Some(Path::new("/home")),
            );
            assert_eq!(got, Some(PathBuf::from("/xdg/QuicMic")));
            let home = resolve_platform_default(None, None, None, Some(Path::new("/home")));
            assert_eq!(home, Some(PathBuf::from("/home/.local/share/QuicMic")));
        }
    }

    #[test]
    fn open_store_explicit_unusable_is_hard_error() {
        // A data dir that is actually a file: `prepare` must fail, and an
        // explicitly configured location must surface the error (hard failure).
        let dir = temp_dir("open-explicit");
        fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("blocker");
        fs::write(&blocker, b"x").unwrap();
        assert!(open_store(DataDir::Explicit(blocker)).is_err());
    }

    #[test]
    fn open_store_default_unusable_degrades_to_ephemeral() {
        let dir = temp_dir("open-default");
        fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("blocker");
        fs::write(&blocker, b"x").unwrap();
        assert!(matches!(
            open_store(DataDir::Default(blocker)).unwrap(),
            StoreResult::Ephemeral
        ));
    }

    #[test]
    fn open_store_no_dir_is_ephemeral() {
        assert!(matches!(
            open_store(DataDir::None).unwrap(),
            StoreResult::Ephemeral
        ));
    }

    // ── PIN parsing ─────────────────────────────────────────────────────

    #[test]
    fn parse_pin_argument_rules() {
        assert!(matches!(parse_pin_arg(None).unwrap(), PinArg::Default));
        assert!(matches!(
            parse_pin_arg(Some("random")).unwrap(),
            PinArg::Random
        ));
        match parse_pin_arg(Some("123456")).unwrap() {
            PinArg::Fixed(p) => assert_eq!(p, "123456"),
            _ => panic!("6 digits must parse as Fixed"),
        }
        for bad in ["1", "12345", "1234567", "abcdef", "12345a", ""] {
            assert!(parse_pin_arg(Some(bad)).is_err(), "must reject {bad:?}");
        }
    }

    // ── PIN store ───────────────────────────────────────────────────────

    #[test]
    fn write_then_read_pin_round_trips() {
        ensure_ring_provider();
        let store = IdentityStore::new(temp_dir("pin-roundtrip"));
        store.prepare().unwrap();
        store.write_pin("314159").unwrap();
        assert_eq!(store.read_pin().unwrap(), "314159");
    }

    #[test]
    fn read_pin_generates_on_first_run_and_persists() {
        let store = IdentityStore::new(temp_dir("pin-first"));
        store.prepare().unwrap();
        let pin = store.read_pin().unwrap();
        assert!(is_valid_pin(&pin));
        // A second store over the same dir sees the same persisted PIN.
        let store2 = IdentityStore::new(store.path().to_path_buf());
        assert_eq!(store2.read_pin().unwrap(), pin);
    }

    #[test]
    fn write_random_pin_persists_and_is_valid() {
        let store = IdentityStore::new(temp_dir("pin-random"));
        store.prepare().unwrap();
        let pin = store.write_random_pin().unwrap();
        assert!(is_valid_pin(&pin));
        assert_eq!(store.read_pin().unwrap(), pin);
    }

    #[test]
    fn write_pin_rejects_invalid_values() {
        let store = IdentityStore::new(temp_dir("pin-invalid"));
        store.prepare().unwrap();
        assert!(store.write_pin("12345").is_err());
        assert!(store.write_pin("abcdef").is_err());
    }

    #[test]
    fn corrupt_pin_is_quarantined_and_regenerated() {
        let dir = temp_dir("pin-corrupt");
        let store = IdentityStore::new(dir.clone());
        store.prepare().unwrap();
        let pin_path = dir.join(PIN_FILE);
        fs::write(&pin_path, "not-a-pin").unwrap();
        let pin = store.read_pin().unwrap();
        assert!(is_valid_pin(&pin));
        assert_ne!(pin, "not-a-pin");
        assert!(fs::read_dir(&dir).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("pin.corrupt-")));
        assert_eq!(store.read_pin().unwrap(), pin);
    }

    #[cfg(unix)]
    #[test]
    fn persisted_files_are_made_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("pin-mode");
        let store = IdentityStore::new(dir.clone());
        store.prepare().unwrap();
        store.write_pin("123456").unwrap();
        let mode = fs::metadata(dir.join(PIN_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "pin file must not be world-readable");
    }

    // ── identity store ──────────────────────────────────────────────────

    #[test]
    fn first_boot_generates_and_persists_identity() {
        ensure_ring_provider();
        let dir = temp_dir("id-first");
        let store = IdentityStore::new(dir.clone());
        store.prepare().unwrap();
        let (_, identity) = store.load_or_create(test_ip(), 1_000_000).unwrap();
        let file: IdentityFile =
            serde_json::from_slice(&fs::read(dir.join(IDENTITY_FILE)).unwrap()).unwrap();
        assert_eq!(file.lan_ip, test_ip().to_string());
        assert_eq!(file.created_at, 1_000_000);
        assert_eq!(
            file.cert_pem, identity.cert_pem,
            "persisted cert must match returned identity"
        );
    }

    #[test]
    fn second_boot_reuses_the_same_identity() {
        ensure_ring_provider();
        let dir = temp_dir("id-reuse");
        let store = IdentityStore::new(dir.clone());
        store.prepare().unwrap();
        let (_, first) = store.load_or_create(test_ip(), 1_000_000).unwrap();
        let store2 = IdentityStore::new(dir);
        let (_, second) = store2.load_or_create(test_ip(), 1_100_000).unwrap();
        assert_eq!(first.cert_hash_base64, second.cert_hash_base64);
        assert_eq!(first.cert_pem, second.cert_pem);
    }

    #[test]
    fn ip_change_regenerates_cert_but_keeps_pin() {
        ensure_ring_provider();
        let dir = temp_dir("id-ipchange");
        let store = IdentityStore::new(dir.clone());
        store.prepare().unwrap();
        store.write_pin("246810").unwrap();
        let (_, first) = store.load_or_create(test_ip(), 1_000_000).unwrap();
        let other_ip: IpAddr = "192.168.1.99".parse().unwrap();
        let (_, second) = store.load_or_create(other_ip, 1_100_000).unwrap();
        assert_ne!(first.cert_hash_base64, second.cert_hash_base64);
        let file: IdentityFile =
            serde_json::from_slice(&fs::read(dir.join(IDENTITY_FILE)).unwrap()).unwrap();
        assert_eq!(file.lan_ip, other_ip.to_string());
        // PIN untouched by identity regeneration.
        assert_eq!(fs::read_to_string(dir.join(PIN_FILE)).unwrap(), "246810");
    }

    #[test]
    fn rotation_happens_at_age_threshold() {
        ensure_ring_provider();
        let dir = temp_dir("id-rotation");
        let store = IdentityStore::new(dir.clone());
        store.prepare().unwrap();
        let (_, first) = store.load_or_create(test_ip(), 1_000_000).unwrap();
        // One second before the threshold: reused.
        let (_, before) = store
            .load_or_create(test_ip(), 1_000_000 + ROTATION_AGE_SECS - 1)
            .unwrap();
        assert_eq!(first.cert_hash_base64, before.cert_hash_base64);
        // At the threshold: rotated.
        let (_, rotated) = store
            .load_or_create(test_ip(), 1_000_000 + ROTATION_AGE_SECS)
            .unwrap();
        assert_ne!(first.cert_hash_base64, rotated.cert_hash_base64);
    }

    #[test]
    fn corrupt_identity_is_quarantined_and_regenerated_pin_untouched() {
        ensure_ring_provider();
        let dir = temp_dir("id-corrupt");
        let store = IdentityStore::new(dir.clone());
        store.prepare().unwrap();
        store.write_pin("135790").unwrap();
        fs::write(dir.join(IDENTITY_FILE), b"this is not json {").unwrap();
        let (_, identity) = store.load_or_create(test_ip(), 1_000_000).unwrap();
        assert!(!identity.cert_hash_base64.is_empty());
        assert!(fs::read_dir(&dir).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("identity.json.corrupt-")));
        assert_eq!(fs::read_to_string(dir.join(PIN_FILE)).unwrap(), "135790");
    }

    #[test]
    fn mismatched_cert_and_key_is_treated_as_corrupt() {
        ensure_ring_provider();
        let dir_a = temp_dir("id-mismatch-a");
        let dir_b = temp_dir("id-mismatch-b");
        let a = IdentityStore::new(dir_a.clone());
        let b = IdentityStore::new(dir_b.clone());
        a.prepare().unwrap();
        b.prepare().unwrap();
        let (_, id_a) = a.load_or_create(test_ip(), 1_000_000).unwrap();
        let (_, id_b) = b.load_or_create(test_ip(), 2_000_000).unwrap();
        // Hand-craft an identity.json whose cert and key come from different
        // generations — the config-build validation must reject it.
        let mixed = IdentityFile {
            cert_pem: id_a.cert_pem.clone(),
            key_pem: id_b.key_pem.clone(),
            lan_ip: test_ip().to_string(),
            // Fresh (well inside the rotation window) so the mismatch check —
            // not the rotation check — is the thing that must reject it.
            created_at: 3_000_000 - 100,
        };
        let dir = temp_dir("id-mismatch");
        let store = IdentityStore::new(dir.clone());
        store.prepare().unwrap();
        fs::write(dir.join(IDENTITY_FILE), serde_json::to_vec(&mixed).unwrap()).unwrap();
        let (_, identity) = store.load_or_create(test_ip(), 3_000_000).unwrap();
        assert!(!identity.cert_hash_base64.is_empty());
        assert!(fs::read_dir(&dir).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("identity.json.corrupt-")));
    }

    #[test]
    fn missing_identity_file_falls_back_to_generation() {
        ensure_ring_provider();
        let dir = temp_dir("id-missing");
        let store = IdentityStore::new(dir.clone());
        store.prepare().unwrap();
        let (_, identity) = store.load_or_create(test_ip(), 1_000_000).unwrap();
        assert!(dir.join(IDENTITY_FILE).exists());
        assert!(!identity.cert_hash_base64.is_empty());
    }

    #[test]
    fn no_tmp_files_left_after_writes() {
        ensure_ring_provider();
        let dir = temp_dir("id-atomic");
        let store = IdentityStore::new(dir.clone());
        store.prepare().unwrap();
        store.write_pin("112233").unwrap();
        store.load_or_create(test_ip(), 1_000_000).unwrap();
        for entry in fs::read_dir(&dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert!(!name.ends_with(".tmp"), "leftover temp file: {name}");
        }
    }

    #[test]
    fn failed_atomic_write_leaves_no_partial_target() {
        ensure_ring_provider();
        let dir = temp_dir("id-atomic-fail");
        let store = IdentityStore::new(dir.clone());
        store.prepare().unwrap();
        // Occupy the temp-file slot with a directory so the atomic write fails
        // mid-flight; the target must never appear (no partial file installed).
        let tmp_slot = dir.join(format!("identity.json.{}.tmp", std::process::id()));
        fs::create_dir_all(&tmp_slot).unwrap();
        assert!(store.load_or_create(test_ip(), 1_000_000).is_err());
        assert!(
            !dir.join(IDENTITY_FILE).exists(),
            "partial identity.json must not be installed"
        );
    }
}
