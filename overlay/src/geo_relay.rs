mod rules;

use crate::{
    starry_config::{self, DatabaseConfig, GeoConfig, MmdbConfig},
    websocket_signal::RelayRequirement,
};
use hbb_common::log;
use maxminddb::{geoip2, Reader};
use once_cell::sync::Lazy;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    net::IpAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, RwLock,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use rules::{DbRequirements, RuleSet};

const MMDB_MARKER: &[u8] = b"\xAB\xCD\xEFMaxMind.com";
const MMDB_MARKER_WINDOW: u64 = 128 * 1024;
const MAX_MMDB_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;

static STATE: Lazy<RwLock<Arc<GeoState>>> =
    Lazy::new(|| RwLock::new(Arc::new(GeoState::disabled())));
static UPDATER_STARTED: AtomicBool = AtomicBool::new(false);
static DOWNLOAD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct GeoState {
    enabled: bool,
    readers: GeoReaders,
    rules: RuleSet,
    warnings: Vec<String>,
}

pub(crate) struct PreparedGeo {
    state: GeoState,
    message: String,
}

#[derive(Clone)]
pub(crate) struct GeoRuntimeSnapshot {
    state: Arc<GeoState>,
}

impl GeoState {
    fn disabled() -> Self {
        Self {
            enabled: false,
            readers: GeoReaders::default(),
            rules: RuleSet::empty(),
            warnings: Vec::new(),
        }
    }

    fn from_config(config: &crate::starry_config::StarryConfig) -> Result<Self, String> {
        if !config.geo.enabled {
            return Ok(Self::disabled());
        }

        let rules = RuleSet::compile(&config.geo)?;
        let readers = GeoReaders::from_config(&config.mmdb)?;
        let warnings = readers.missing_requirements(rules.requirements());
        Ok(Self {
            enabled: true,
            readers,
            rules,
            warnings,
        })
    }
}

#[derive(Default)]
struct GeoReaders {
    country: Option<Reader<Vec<u8>>>,
    city: Option<Reader<Vec<u8>>>,
    asn: Option<Reader<Vec<u8>>>,
    loaded: Vec<String>,
}

impl GeoReaders {
    fn from_config(config: &MmdbConfig) -> Result<Self, String> {
        let country = open_optional_reader("Country", &config.country.path)?;
        let city = open_optional_reader("City", &config.city.path)?;
        let asn = open_optional_reader("ASN", &config.asn.path)?;
        let mut loaded = Vec::new();
        if country.is_some() {
            loaded.push(format!("Country={}", config.country.path));
        }
        if city.is_some() {
            loaded.push(format!("City={}", config.city.path));
        }
        if asn.is_some() {
            loaded.push(format!("ASN={}", config.asn.path));
        }
        Ok(Self {
            country,
            city,
            asn,
            loaded,
        })
    }

    fn lookup(&self, ip: IpAddr) -> GeoFacts {
        let mut facts = GeoFacts::default();
        if let Some(reader) = self.city.as_ref() {
            lookup_city(reader, ip, &mut facts);
        }
        if let Some(reader) = self.country.as_ref() {
            lookup_country(reader, ip, &mut facts);
        }
        if let Some(reader) = self.asn.as_ref() {
            lookup_asn(reader, ip, &mut facts);
        }
        facts
    }

    fn missing_requirements(&self, requirements: DbRequirements) -> Vec<String> {
        let mut warnings = Vec::new();
        if requirements.city && self.city.is_none() {
            warnings.push("rules use City fields but the City MMDB is unavailable".to_owned());
        }
        if requirements.asn && self.asn.is_none() {
            warnings.push("rules use ASN/ISP fields but the ASN MMDB is unavailable".to_owned());
        }
        if requirements.country && self.country.is_none() && self.city.is_none() {
            warnings.push(
                "rules use Country/Continent fields but neither Country nor City MMDB is available"
                    .to_owned(),
            );
        }
        warnings
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct GeoFacts {
    pub(super) continent: Option<String>,
    pub(super) country: Option<String>,
    pub(super) subdivision_codes: Vec<String>,
    pub(super) subdivision_names: Vec<String>,
    pub(super) city_names: Vec<String>,
    pub(super) city_geoname_id: Option<u32>,
    pub(super) asn: Option<u32>,
    pub(super) asn_org: Option<String>,
}

pub(crate) fn validate_config(config: &GeoConfig) -> Result<(), String> {
    RuleSet::compile(config).map(|_| ())
}

#[derive(Clone, Debug)]
pub(crate) struct GeoRelaySelection {
    pub(crate) relay: String,
    pub(crate) candidates: Vec<String>,
    pub(crate) rule_name: String,
    pub(crate) rule_index: usize,
    pub(crate) direction: &'static str,
}

pub fn reload() -> String {
    let Some(config) = starry_config::snapshot() else {
        return replace_state(
            GeoState::disabled(),
            "Geo relay disabled because Starry config is unavailable; using upstream relay selection"
                .to_owned(),
        );
    };

    match prepare(&config).and_then(activate_prepared) {
        Ok(ack) => ack.detail,
        Err(err) => format!("Geo relay reload rejected; retained last-known-good state: {err}"),
    }
}

pub(crate) fn prepare(config: &crate::starry_config::StarryConfig) -> Result<PreparedGeo, String> {
    let state = GeoState::from_config(config)?;
    let enabled = state.enabled;
    let rule_count = state.rules.len();
    let databases = if state.readers.loaded.is_empty() {
        "none".to_owned()
    } else {
        state.readers.loaded.join(", ")
    };
    let warnings = state.warnings.clone();
    let message = if enabled {
        let mut message =
            format!("Geo relay loaded: {rule_count} ordered rules, databases={databases}");
        if !warnings.is_empty() {
            message.push_str(&format!("; warnings: {}", warnings.join("; ")));
        }
        message
    } else {
        "Geo relay disabled by Starry config; using upstream relay selection".to_owned()
    };
    Ok(PreparedGeo { state, message })
}

pub(crate) fn activate_prepared(
    prepared: PreparedGeo,
) -> Result<starry_config::SubsystemAck, String> {
    let mut state = STATE
        .write()
        .map_err(|err| format!("Geo relay state lock failed: {err}"))?;
    *state = Arc::new(prepared.state);
    Ok(starry_config::SubsystemAck {
        subsystem: "geo".to_owned(),
        accepted: true,
        detail: prepared.message,
    })
}

pub fn select_relay(
    pa: IpAddr,
    pb: IpAddr,
    eligible_relays: &[String],
    requirement: RelayRequirement,
) -> Option<String> {
    select_relay_explained(pa, pb, eligible_relays, requirement).map(|selection| selection.relay)
}

pub(crate) fn select_relay_explained(
    pa: IpAddr,
    pb: IpAddr,
    eligible_relays: &[String],
    requirement: RelayRequirement,
) -> Option<GeoRelaySelection> {
    let snapshot = runtime_snapshot();
    select_relay_explained_from(&snapshot, pa, pb, eligible_relays, requirement)
}

pub(crate) fn runtime_snapshot() -> GeoRuntimeSnapshot {
    let state = STATE
        .read()
        .map(|state| state.clone())
        .unwrap_or_else(|_| Arc::new(GeoState::disabled()));
    GeoRuntimeSnapshot { state }
}

pub(crate) fn select_relay_explained_from(
    snapshot: &GeoRuntimeSnapshot,
    pa: IpAddr,
    pb: IpAddr,
    eligible_relays: &[String],
    requirement: RelayRequirement,
) -> Option<GeoRelaySelection> {
    let state = snapshot.state.as_ref();
    if !state.enabled || eligible_relays.is_empty() {
        return None;
    }

    let facts_a = state.readers.lookup(pa);
    let facts_b = state.readers.lookup(pb);
    let selection = state.rules.select(&facts_a, &facts_b, eligible_relays)?;
    log::debug!(
        "Geo relay selected {} by rule '{}' for requirement {:?}",
        selection.relay,
        selection.rule_name,
        requirement
    );
    Some(GeoRelaySelection {
        relay: selection.relay,
        candidates: selection.candidates,
        rule_name: selection.rule_name,
        rule_index: selection.rule_index,
        direction: selection.direction,
    })
}

pub fn start_mmdb_updater() -> String {
    if UPDATER_STARTED.swap(true, Ordering::SeqCst) {
        return "MMDB updater already started".to_owned();
    }
    match thread::Builder::new()
        .name("starry-mmdb-updater".to_owned())
        .spawn(mmdb_update_loop)
    {
        Ok(_) => "MMDB updater thread started".to_owned(),
        Err(err) => {
            UPDATER_STARTED.store(false, Ordering::SeqCst);
            format!("cannot start MMDB updater thread: {err}")
        }
    }
}

fn replace_state(new_state: GeoState, message: String) -> String {
    match STATE.write() {
        Ok(mut state) => {
            *state = Arc::new(new_state);
            message
        }
        Err(err) => format!("Geo relay state lock failed: {err}"),
    }
}

fn open_optional_reader(label: &str, path: &str) -> Result<Option<Reader<Vec<u8>>>, String> {
    if !Path::new(path).is_file() {
        return Ok(None);
    }
    Reader::open_readfile(path)
        .map(Some)
        .map_err(|err| format!("cannot open {label} MMDB {path}: {err}"))
}

fn lookup_city(reader: &Reader<Vec<u8>>, ip: IpAddr, facts: &mut GeoFacts) {
    let Ok(result) = reader.lookup(ip) else {
        return;
    };
    let Ok(Some(record)) = result.decode::<geoip2::City>() else {
        return;
    };

    set_if_empty(&mut facts.continent, record.continent.code);
    set_if_empty(&mut facts.country, record.country.iso_code);
    facts.city_geoname_id = record.city.geoname_id;
    append_names(&mut facts.city_names, &record.city.names);
    for subdivision in record.subdivisions {
        if let Some(code) = subdivision.iso_code {
            push_unique(&mut facts.subdivision_codes, code);
        }
        append_names(&mut facts.subdivision_names, &subdivision.names);
    }
}

fn lookup_country(reader: &Reader<Vec<u8>>, ip: IpAddr, facts: &mut GeoFacts) {
    let Ok(result) = reader.lookup(ip) else {
        return;
    };
    let Ok(Some(record)) = result.decode::<geoip2::Country>() else {
        return;
    };
    set_if_empty(&mut facts.continent, record.continent.code);
    set_if_empty(&mut facts.country, record.country.iso_code);
}

fn lookup_asn(reader: &Reader<Vec<u8>>, ip: IpAddr, facts: &mut GeoFacts) {
    let Ok(result) = reader.lookup(ip) else {
        return;
    };
    let Ok(Some(record)) = result.decode::<geoip2::Asn>() else {
        return;
    };
    facts.asn = record.autonomous_system_number;
    facts.asn_org = record
        .autonomous_system_organization
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
}

fn append_names(target: &mut Vec<String>, names: &geoip2::Names<'_>) {
    for name in [
        names.english,
        names.simplified_chinese,
        names.japanese,
        names.german,
        names.spanish,
        names.french,
        names.brazilian_portuguese,
        names.russian,
    ]
    .into_iter()
    .flatten()
    {
        push_unique(target, name);
    }
}

fn push_unique(target: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if !value.is_empty() && !target.iter().any(|old| old.eq_ignore_ascii_case(value)) {
        target.push(value.to_owned());
    }
}

fn set_if_empty(target: &mut Option<String>, value: Option<&str>) {
    if target.is_none() {
        *target = value
            .map(|value| value.trim().to_ascii_uppercase())
            .filter(|value| !value.is_empty());
    }
}

fn mmdb_update_loop() {
    let mut active_config: Option<MmdbConfig> = None;
    let mut next_update: Option<Instant> = None;
    loop {
        if let Some(config) = starry_config::snapshot() {
            let mmdb = config.mmdb.clone();
            if active_config.as_ref() != Some(&mmdb) {
                if mmdb.update_on_start {
                    update_and_reload(&mmdb, mmdb.force_update);
                }
                next_update = next_update_at(&mmdb);
                active_config = Some(mmdb);
            } else if next_update
                .map(|deadline| Instant::now() >= deadline)
                .unwrap_or(false)
            {
                update_and_reload(&mmdb, mmdb.force_update);
                next_update = next_update_at(&mmdb);
            }
        } else {
            active_config = None;
            next_update = None;
        }
        thread::sleep(Duration::from_secs(60));
    }
}

fn next_update_at(config: &MmdbConfig) -> Option<Instant> {
    if config.update_interval_hours == 0 {
        None
    } else {
        Instant::now().checked_add(Duration::from_secs(
            config.update_interval_hours.saturating_mul(3_600),
        ))
    }
}

fn update_and_reload(config: &MmdbConfig, force: bool) {
    let (updated, errors) = update_all(config, force);
    if updated > 0 {
        let reload_status = reload();
        log::info!(
            "Starry MMDB update finished: {updated} database(s); {}",
            reload_status
        );
    } else if errors.is_empty() {
        log::debug!("Starry MMDB update check finished: no changes");
    }
    if !errors.is_empty() {
        log::error!(
            "Starry MMDB update failed for {} database(s); failed databases kept their existing files: {}",
            errors.len(),
            errors.join("; ")
        );
    }
}

fn update_all(config: &MmdbConfig, force: bool) -> (usize, Vec<String>) {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(config.download_timeout_seconds))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(err) => return (0, vec![format!("cannot create HTTP client: {err}")]),
    };
    let interval = Duration::from_secs(config.update_interval_hours.saturating_mul(3_600));
    let mut updated = 0;
    let mut errors = Vec::new();
    for (label, database) in [
        ("Country", &config.country),
        ("City", &config.city),
        ("ASN", &config.asn),
    ] {
        if database.url.is_empty() || !database_due(&database.path, interval, force) {
            continue;
        }
        match download_database(&client, label, database, config.minimum_bytes) {
            Ok(()) => updated += 1,
            Err(err) => errors.push(err),
        }
    }
    (updated, errors)
}

fn database_due(path: &str, interval: Duration, force: bool) -> bool {
    if force || !Path::new(path).is_file() {
        return true;
    }
    if interval.is_zero() {
        return false;
    }
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .map(|age| age >= interval)
        .unwrap_or(true)
}

fn download_database(
    client: &reqwest::blocking::Client,
    label: &str,
    database: &DatabaseConfig,
    minimum_bytes: u64,
) -> Result<(), String> {
    let target = Path::new(&database.path);
    reject_symlink_components(target)?;
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
    let temporary = temporary_path(target);
    let result = download_to_temporary(client, label, database, &temporary, minimum_bytes)
        .and_then(|()| replace_database(&temporary, target));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn download_to_temporary(
    client: &reqwest::blocking::Client,
    label: &str,
    database: &DatabaseConfig,
    temporary: &Path,
    minimum_bytes: u64,
) -> Result<(), String> {
    log::info!("Downloading Starry {label} MMDB");
    let response = client
        .get(&database.url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|err| {
            let detail = if let Some(status) = err.status() {
                format!("HTTP {status}")
            } else if err.is_timeout() {
                "request timed out".to_owned()
            } else if err.is_connect() {
                "connection failed".to_owned()
            } else {
                "request failed".to_owned()
            };
            format!("{label} download failed: {detail}")
        })?;
    if response
        .content_length()
        .map(|length| length > MAX_MMDB_DOWNLOAD_BYTES)
        .unwrap_or(false)
    {
        return Err(format!(
            "{label} download exceeds the {MAX_MMDB_DOWNLOAD_BYTES}-byte limit"
        ));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary)
        .map_err(|err| format!("cannot create {}: {err}", temporary.display()))?;
    let mut limited = response.take(MAX_MMDB_DOWNLOAD_BYTES + 1);
    std::io::copy(&mut limited, &mut file)
        .map_err(|err| format!("cannot write {}: {err}", temporary.display()))?;
    file.flush()
        .map_err(|err| format!("cannot flush {}: {err}", temporary.display()))?;
    let size = file
        .metadata()
        .map_err(|err| format!("cannot inspect {}: {err}", temporary.display()))?
        .len();
    drop(file);
    if size > MAX_MMDB_DOWNLOAD_BYTES {
        return Err(format!(
            "{label} download exceeds the {MAX_MMDB_DOWNLOAD_BYTES}-byte limit"
        ));
    }
    if size < minimum_bytes {
        return Err(format!(
            "{label} download is too small: {size} bytes, expected at least {minimum_bytes}"
        ));
    }
    if !contains_mmdb_marker(temporary)? {
        return Err(format!(
            "{label} download has no MaxMind DB metadata marker"
        ));
    }
    Reader::open_readfile(temporary)
        .map(|_: Reader<Vec<u8>>| ())
        .map_err(|err| format!("{label} download is not a readable MMDB: {err}"))
}

fn contains_mmdb_marker(path: &Path) -> Result<bool, String> {
    let mut file = File::open(path)
        .map_err(|err| format!("cannot open {} for validation: {err}", path.display()))?;
    let size = file
        .metadata()
        .map_err(|err| format!("cannot inspect {}: {err}", path.display()))?
        .len();
    let window = size.min(MMDB_MARKER_WINDOW);
    file.seek(SeekFrom::End(-(window as i64)))
        .map_err(|err| format!("cannot seek {}: {err}", path.display()))?;
    let mut tail = Vec::with_capacity(window as usize);
    file.read_to_end(&mut tail)
        .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    Ok(tail
        .windows(MMDB_MARKER.len())
        .any(|window| window == MMDB_MARKER))
}

fn temporary_path(target: &Path) -> PathBuf {
    let sequence = DOWNLOAD_SEQUENCE.fetch_add(1, Ordering::SeqCst);
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("database.mmdb");
    target.with_file_name(format!(
        ".{file_name}.download.{}.{}",
        std::process::id(),
        sequence
    ))
}

fn reject_symlink_components(target: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in target.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "MMDB path component {} must not be a symbolic link",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(format!(
                    "cannot inspect MMDB path component {}: {err}",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn replace_database(temporary: &Path, target: &Path) -> Result<(), String> {
    fs::rename(temporary, target).map_err(|err| {
        format!(
            "cannot atomically replace {} with {}: {err}",
            target.display(),
            temporary.display()
        )
    })
}

#[cfg(target_os = "windows")]
fn replace_database(temporary: &Path, target: &Path) -> Result<(), String> {
    let backup = target.with_extension("mmdb.previous");
    if backup.exists() {
        fs::remove_file(&backup)
            .map_err(|err| format!("cannot remove stale {}: {err}", backup.display()))?;
    }
    if target.exists() {
        fs::rename(target, &backup)
            .map_err(|err| format!("cannot stage {} for replacement: {err}", target.display()))?;
    }
    if let Err(err) = fs::rename(temporary, target) {
        if backup.exists() {
            let _ = fs::rename(&backup, target);
        }
        return Err(format!("cannot replace {}: {err}", target.display()));
    }
    if backup.exists() {
        let _ = fs::remove_file(backup);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_mmdb_marker_only_in_the_tail_window() {
        let directory =
            std::env::temp_dir().join(format!("starry-mmdb-marker-{}", std::process::id()));
        let path = directory.join("test.mmdb");
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        let mut file = File::create(&path).unwrap();
        file.write_all(&vec![0; 70 * 1024]).unwrap();
        file.write_all(MMDB_MARKER).unwrap();
        drop(file);
        assert!(contains_mmdb_marker(&path).unwrap());
        let _ = fs::remove_dir_all(directory);
    }
}
