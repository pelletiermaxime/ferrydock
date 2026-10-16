//! Linux AppImage updates. All network, hashing and filesystem work stays on one
//! serial worker. Release checksums provide integrity, not publisher signatures.
use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::Client;
use ring::digest::{Context as Digest, SHA256};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::Command as Process,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const REPOSITORY: &str = "pelletiermaxime/ferrydock";
const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const MAX_IMAGE: u64 = 512 * 1024 * 1024;
const MAX_RELEASE: u64 = 1024 * 1024;
const MAX_CHECKSUMS: u64 = 64 * 1024;

#[derive(Debug)]
pub enum Command {
    Check,
    Automatic(bool),
    Apply,
}

#[derive(Debug)]
pub enum Event {
    Loaded {
        automatic: bool,
        unavailable: Option<String>,
    },
    Automatic(bool),
    Checking,
    Downloading {
        version: String,
        bytes: u64,
        total: u64,
    },
    Ready(String),
    Current,
    Error(String),
    Relaunched,
}

pub struct Worker {
    pub commands: mpsc::SyncSender<Command>,
    pub events: mpsc::Receiver<Event>,
    cancelled: Arc<AtomicBool>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}
impl Worker {
    pub fn start() -> Self {
        let (commands, requests) = mpsc::sync_channel(1);
        let (sender, events) = mpsc::sync_channel(8);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        std::thread::spawn(move || run(requests, sender, stop));
        Self {
            commands,
            events,
            cancelled,
        }
    }

    #[cfg(all(test, feature = "headless-tests"))]
    pub(crate) fn fixture() -> (Self, mpsc::Receiver<Command>, mpsc::SyncSender<Event>) {
        let (commands, requests) = mpsc::sync_channel(1);
        let (sender, events) = mpsc::sync_channel(8);
        (
            Self {
                commands,
                events,
                cancelled: Arc::new(AtomicBool::new(false)),
            },
            requests,
            sender,
        )
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Preferences {
    automatic: bool,
    last_check: u64,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            automatic: true,
            last_check: 0,
        }
    }
}
impl Preferences {
    fn load(path: &Path) -> Result<Self> {
        match File::open(path) {
            Ok(file) => Ok(serde_json::from_slice(&bounded(file, 4096)?)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }
    fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().context("Settings directory unavailable")?;
        fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&serde_json::to_vec_pretty(self)?)?;
        temporary.as_file().sync_all()?;
        temporary.persist(path)?;
        Ok(())
    }
    fn due(&self, now: u64) -> bool {
        self.automatic
            && (now < self.last_check || now.saturating_sub(self.last_check) >= INTERVAL.as_secs())
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, PartialEq, Eq)]
struct Identity {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
}
fn identity(path: &Path) -> Result<Identity> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file(),
        "The AppImage path is no longer a regular file"
    );
    Ok(Identity {
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.len(),
        modified: (metadata.mtime(), metadata.mtime_nsec()),
    })
}

struct Installation {
    path: PathBuf,
    identity: Identity,
}
impl Installation {
    fn detect() -> Result<Self> {
        ensure!(
            cfg!(target_arch = "x86_64"),
            "Automatic updates currently support Linux x86-64 AppImages"
        );
        let image = std::env::var_os("APPIMAGE").context("In-app installation is available for AppImages. Download an AppImage from GitHub Releases to use it.")?;
        let appdir = std::env::var_os("APPDIR")
            .context("The AppImage runtime did not provide its application directory")?;
        ensure!(
            fs::canonicalize(std::env::current_exe()?)?
                == fs::canonicalize(Path::new(&appdir).join("usr/bin/ferrydock"))?,
            "This process was not launched from the AppImage application directory"
        );
        let path = fs::canonicalize(image).context("Cannot find the original AppImage")?;
        validate_header(&mut File::open(&path)?)?;
        let identity = identity(&path)?;
        Ok(Self { path, identity })
    }
    fn unchanged(&self) -> Result<()> {
        ensure!(
            identity(&self.path)? == self.identity,
            "The installed AppImage changed; restart FerryDock before updating"
        );
        Ok(())
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Clone, Deserialize)]
struct Asset {
    name: String,
    size: u64,
    browser_download_url: String,
}
struct Candidate {
    version: String,
    image: Asset,
    checksums: Asset,
}

fn select_release(bytes: &[u8], current: &str) -> Result<Option<Candidate>> {
    let release: Release =
        serde_json::from_slice(bytes).context("Cannot read GitHub release information")?;
    ensure!(
        !release.draft && !release.prerelease,
        "Only stable published releases can be installed"
    );
    let version = release
        .tag_name
        .strip_prefix('v')
        .context("Unexpected release tag")?;
    let parsed = Version::parse(version)?;
    ensure!(
        parsed.pre.is_empty() && parsed.build.is_empty(),
        "Only stable release versions can be installed"
    );
    if parsed <= Version::parse(current)? {
        return Ok(None);
    }
    ensure!(release.assets.len() <= 256, "Too many release assets");
    let asset = |name: &str, limit: u64| -> Result<Asset> {
        let matches = release
            .assets
            .iter()
            .filter(|asset| asset.name == name)
            .collect::<Vec<_>>();
        ensure!(
            matches.len() == 1,
            "The release must contain exactly one {name}"
        );
        let asset = matches[0];
        ensure!(
            asset.size > 0 && asset.size <= limit,
            "Unexpected size for {name}"
        );
        ensure!(
            asset.browser_download_url
                == format!(
                    "https://github.com/{REPOSITORY}/releases/download/{}/{name}",
                    release.tag_name
                ),
            "Unexpected release download URL"
        );
        Ok(asset.clone())
    };
    Ok(Some(Candidate {
        version: version.into(),
        image: asset(
            &format!("ferrydock-{version}-linux-x86_64.AppImage"),
            MAX_IMAGE,
        )?,
        checksums: asset("SHA256SUMS", MAX_CHECKSUMS)?,
    }))
}

fn bounded(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Update response exceeds its size limit"
    );
    Ok(bytes)
}

fn checksum(bytes: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut found = None;
    for line in std::str::from_utf8(bytes)?.lines() {
        // Match sha256sum's text and binary formats, with an exact filename.
        if line.len() >= 66
            && line.is_char_boundary(64)
            && line.is_char_boundary(66)
            && matches!(&line[64..66], "  " | " *")
            && &line[66..] == name
        {
            ensure!(found.is_none(), "Duplicate checksum for the AppImage");
            let hex = &line[..64];
            ensure!(
                hex.bytes().all(|b| b.is_ascii_hexdigit()),
                "Invalid AppImage SHA-256 checksum"
            );
            found = Some(
                (0..64)
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                    .collect(),
            );
        }
    }
    found.context("The release has no SHA-256 checksum for this AppImage")
}

fn validate_header(file: &mut File) -> Result<()> {
    let mut header = [0; 12];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut header)?;
    ensure!(
        &header[..4] == b"\x7fELF" && &header[8..11] == b"AI\x02",
        "The download is not a type-2 AppImage"
    );
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

struct Pending {
    file: tempfile::NamedTempFile,
    version: String,
    size: u64,
    checksum: Vec<u8>,
    workspace: Option<Workspace>,
}

// A stable private directory and an OS lock bound abandoned downloads after a
// crash or process exit. A later download removes only this installation's
// stale staging files, and never another running instance's candidate.
struct Workspace {
    path: PathBuf,
    _lock: File,
}
impl Workspace {
    fn acquire(installation: &Installation) -> Result<Self> {
        let hash = ring::digest::digest(&SHA256, installation.path.as_os_str().as_bytes());
        let suffix: String = hash.as_ref()[..12]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let path = installation
            .path
            .parent()
            .context("AppImage directory unavailable")?
            .join(format!(".ferrydock-updates-{suffix}"));
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error).context("Cannot stage the update beside the AppImage; check free space and directory permissions"),
        }
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            metadata.is_dir()
                && metadata.uid() == fs::metadata("/proc/self")?.uid()
                && metadata.permissions().mode() & 0o777 == 0o700,
            "The update staging directory must be private and owned by your user"
        );
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.join("lock"))?;
        lock.try_lock()
            .context("Another FerryDock instance has a pending update")?;
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("candidate-"))
                && entry.file_type()?.is_file()
            {
                fs::remove_file(entry.path())?;
            }
        }
        Ok(Self { path, _lock: lock })
    }
}
impl Pending {
    fn verify(&mut self) -> Result<()> {
        ensure!(
            self.file.as_file().metadata()?.len() == self.size,
            "AppImage download size does not match the release"
        );
        validate_header(self.file.as_file_mut())?;
        let mut hash = Digest::new(&SHA256);
        let mut buffer = [0; 64 * 1024];
        loop {
            let count = self.file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        ensure!(
            hash.finish().as_ref() == self.checksum,
            "AppImage SHA-256 verification failed; the installed application was preserved"
        );
        Ok(())
    }
}

struct Source {
    client: Client,
    latest: String,
}
impl Source {
    fn github() -> Result<Self> {
        let client = Client::builder()
            .https_only(true)
            .user_agent(concat!("FerryDock/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(600))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;
        Ok(Self {
            client,
            latest: format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"),
        })
    }
    fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>> {
        bounded(
            self.client
                .get(url)
                .timeout(Duration::from_secs(30))
                .header("Accept", "application/vnd.github+json")
                .send()?
                .error_for_status()?,
            limit,
        )
    }
    fn download(
        &self,
        candidate: Candidate,
        installation: &Installation,
        events: &mpsc::SyncSender<Event>,
        cancelled: &AtomicBool,
    ) -> Result<Pending> {
        installation.unchanged()?;
        let expected = checksum(
            &self.get(&candidate.checksums.browser_download_url, MAX_CHECKSUMS)?,
            &candidate.image.name,
        )?;
        let mut response = self
            .client
            .get(&candidate.image.browser_download_url)
            .send()?
            .error_for_status()?;
        if let Some(length) = response.content_length() {
            ensure!(
                length == candidate.image.size,
                "AppImage download length does not match the release"
            );
        }
        let workspace = Workspace::acquire(installation)?;
        let mut file = tempfile::Builder::new()
            .prefix("candidate-")
            .suffix(".AppImage")
            .tempfile_in(&workspace.path)?;
        let mut bytes = 0;
        let mut reported = Instant::now();
        let mut buffer = [0; 64 * 1024];
        let progress = |bytes| Event::Downloading {
            version: candidate.version.clone(),
            bytes,
            total: candidate.image.size,
        };
        let _ = events.send(progress(0));
        loop {
            ensure!(
                !cancelled.load(Ordering::Relaxed),
                "Update download cancelled"
            );
            let count = response.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            ensure!(
                bytes <= candidate.image.size,
                "AppImage download exceeds the release size"
            );
            file.write_all(&buffer[..count])?;
            if reported.elapsed() >= Duration::from_millis(250) {
                let _ = events.try_send(progress(bytes));
                reported = Instant::now();
            }
        }
        file.as_file().sync_all()?;
        let mut pending = Pending {
            file,
            version: candidate.version,
            size: candidate.image.size,
            checksum: expected,
            workspace: Some(workspace),
        };
        pending.verify()?;
        Ok(pending)
    }
}

/// Preserve the original inode until launching succeeds. Never overwrite the
/// executable inside APPDIR: that is a mounted or extracted runtime directory.
fn install(
    mut pending: Pending,
    installation: &Installation,
    launch: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    pending.verify()?;
    installation.unchanged()?;
    let parent = installation
        .path
        .parent()
        .context("AppImage directory unavailable")?;
    let backup = tempfile::Builder::new()
        .prefix(".ferrydock-rollback-")
        .tempdir_in(parent)?;
    let previous = backup.path().join("previous.AppImage");
    fs::hard_link(&installation.path, &previous)
        .context("Cannot preserve the current AppImage for rollback")?;
    pending
        .file
        .as_file()
        .set_permissions(fs::Permissions::from_mode(0o755))?;
    pending.file.persist(&installation.path)?;
    // The relaunched process may immediately acquire the same staging lock.
    drop(pending.workspace);
    if let Err(error) = launch(&installation.path) {
        if let Err(rollback) = fs::rename(&previous, &installation.path) {
            let retained = backup.keep();
            bail!(
                "{error:#}. Rollback failed: {rollback}. Previous AppImage retained at {}",
                retained.display()
            );
        }
        return Err(error.context("Could not restart; restored the previous AppImage"));
    }
    Ok(())
}

fn relaunch(path: &Path) -> Result<()> {
    let mut command = Process::new(path);
    // The new runtime establishes its own mount and library search paths.
    for name in ["APPIMAGE", "APPDIR", "ARGV0", "OWD", "LD_LIBRARY_PATH"] {
        command.env_remove(name);
    }
    if let Some(original) = std::env::var_os("FERRYDOCK_ORIGINAL_LD_LIBRARY_PATH") {
        command.env("LD_LIBRARY_PATH", original);
    }
    command.env("APPIMAGE_EXTRACT_AND_RUN", "1");
    command.args(std::env::args_os().skip(1));
    let mut child = command
        .spawn()
        .context("Cannot launch the updated AppImage")?;
    // Detect immediate runtime/loader errors while the old UI can still report
    // failure. This is a launch check, not a claim of a healthy new window.
    std::thread::sleep(Duration::from_millis(750));
    ensure!(
        child.try_wait()?.is_none(),
        "The updated AppImage exited during startup"
    );
    Ok(())
}

fn run(
    requests: mpsc::Receiver<Command>,
    events: mpsc::SyncSender<Event>,
    cancelled: Arc<AtomicBool>,
) {
    let installation = Installation::detect();
    let settings = crate::compose::Settings::path().map(|path| path.with_file_name("updates.json"));
    let loaded = settings
        .as_ref()
        .map_err(|e| anyhow::anyhow!("{e:#}"))
        .and_then(|path| Preferences::load(path));
    let (mut preferences, settings_error) = match loaded {
        Ok(prefs) => (prefs, None),
        Err(error) => (
            Preferences {
                automatic: false,
                last_check: 0,
            },
            Some(format!("Cannot load update settings: {error:#}")),
        ),
    };
    let unavailable = installation.as_ref().err().map(|e| format!("{e:#}"));
    if events
        .send(Event::Loaded {
            automatic: preferences.automatic,
            unavailable,
        })
        .is_err()
    {
        return;
    }
    if let Some(error) = settings_error {
        let _ = events.send(Event::Error(error));
    }
    if installation.is_err() {
        return;
    }
    let mut pending: Option<Pending> = None;
    let started = Instant::now();
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return;
        }
        let request = match requests.recv_timeout(Duration::from_secs(1)) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if installation.is_ok()
                    && pending.is_none()
                    && started.elapsed() >= Duration::from_secs(10)
                    && preferences.due(now())
                {
                    Command::Check
                } else {
                    continue;
                }
            }
        };
        let result: Result<()> = (|| {
            match request {
                Command::Automatic(enabled) => {
                    let previous = preferences.automatic;
                    preferences.automatic = enabled;
                    if let Err(error) = settings
                        .as_ref()
                        .map_err(|e| anyhow::anyhow!("{e:#}"))
                        .and_then(|p| preferences.save(p))
                    {
                        preferences.automatic = previous;
                        return Err(error.context("Cannot save automatic update preference"));
                    }
                    let _ = events.send(Event::Automatic(enabled));
                }
                Command::Check => {
                    if let Some(update) = &pending {
                        let _ = events.send(Event::Ready(update.version.clone()));
                        return Ok(());
                    }
                    let installation = installation
                        .as_ref()
                        .map_err(|e| anyhow::anyhow!("{e:#}"))?;
                    preferences.last_check = now();
                    // Keep the in-memory backoff even if preferences cannot be saved.
                    if let Ok(path) = &settings {
                        preferences.save(path)?;
                    }
                    let _ = events.send(Event::Checking);
                    let source = Source::github()?;
                    if let Some(candidate) = select_release(
                        &source.get(&source.latest, MAX_RELEASE)?,
                        env!("CARGO_PKG_VERSION"),
                    )? {
                        let update =
                            source.download(candidate, installation, &events, &cancelled)?;
                        let _ = events.send(Event::Ready(update.version.clone()));
                        pending = Some(update);
                    } else {
                        let _ = events.send(Event::Current);
                    }
                }
                Command::Apply => {
                    let installation = installation
                        .as_ref()
                        .map_err(|e| anyhow::anyhow!("{e:#}"))?;
                    let update = pending
                        .take()
                        .context("Check for updates before restarting")?;
                    install(update, installation, relaunch)?;
                    let _ = events.send(Event::Relaunched);
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            let _ = events.send(Event::Error(format!("{error:#}")));
        }
    }
}

#[cfg(test)]
mod tests;
