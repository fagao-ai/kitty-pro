use crate::CoreError;
use libloading::Library;
use std::ffi::OsString;
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

type ProbeFn = unsafe extern "C" fn(*const c_char, *const c_char, *const c_char) -> *mut c_char;
type ProbeOutboundFn = unsafe extern "C" fn(u64, *const c_char, *const c_char) -> *mut c_char;
type StartFn = unsafe extern "C" fn(*const c_char) -> u64;
type StopFn = unsafe extern "C" fn(u64) -> i32;
type StringFn = unsafe extern "C" fn() -> *mut c_char;
type TrafficFn = unsafe extern "C" fn(u64) -> *mut c_char;
type LogsFn = unsafe extern "C" fn(u64, u64) -> *mut c_char;
type SetLogEnabledFn = unsafe extern "C" fn(u64, i32) -> i32;
type PathFn = unsafe extern "C" fn(*const c_char) -> *mut c_char;
type SelectOutboundFn = unsafe extern "C" fn(u64, *const c_char, *const c_char) -> i32;
type FreeStringFn = unsafe extern "C" fn(*mut c_char);

pub(crate) struct Bridge {
    _library: Library,
    pub(crate) probe: ProbeFn,
    pub(crate) probe_outbound: ProbeOutboundFn,
    pub(crate) start: StartFn,
    pub(crate) stop: StopFn,
    pub(crate) version: StringFn,
    pub(crate) last_error: StringFn,
    pub(crate) traffic: TrafficFn,
    pub(crate) logs: LogsFn,
    pub(crate) set_log_enabled: SetLogEnabledFn,
    pub(crate) validate_rule_set_file: PathFn,
    pub(crate) check_config: PathFn,
    pub(crate) select_outbound: SelectOutboundFn,
    pub(crate) free_string: FreeStringFn,
}

impl Bridge {
    fn load() -> Result<Self, String> {
        let candidates = bridge_candidates();
        let mut errors = Vec::new();
        for path in candidates {
            match unsafe { Self::load_from(&path) } {
                Ok(bridge) => return Ok(bridge),
                Err(error) => errors.push(format!("{}: {error}", path.display())),
            }
        }
        Err(format!(
            "failed to load kitty_singbox.dll ({})",
            errors.join("; ")
        ))
    }

    unsafe fn load_from(path: &Path) -> Result<Self, String> {
        let library = unsafe { Library::new(path) }.map_err(|error| error.to_string())?;
        let probe = unsafe { load_symbol(&library, b"kitty_singbox_probe\0")? };
        let probe_outbound = unsafe { load_symbol(&library, b"kitty_singbox_probe_outbound\0")? };
        let start = unsafe { load_symbol(&library, b"kitty_singbox_start\0")? };
        let stop = unsafe { load_symbol(&library, b"kitty_singbox_stop\0")? };
        let version = unsafe { load_symbol(&library, b"kitty_singbox_version\0")? };
        let last_error = unsafe { load_symbol(&library, b"kitty_singbox_last_error\0")? };
        let traffic = unsafe { load_symbol(&library, b"kitty_singbox_traffic\0")? };
        let logs = unsafe { load_symbol(&library, b"kitty_singbox_logs\0")? };
        let set_log_enabled = unsafe { load_symbol(&library, b"kitty_singbox_set_log_enabled\0")? };
        let validate_rule_set_file =
            unsafe { load_symbol(&library, b"kitty_singbox_validate_rule_set_file\0")? };
        let check_config = unsafe { load_symbol(&library, b"kitty_singbox_check_config\0")? };
        let select_outbound = unsafe { load_symbol(&library, b"kitty_singbox_select_outbound\0")? };
        let free_string = unsafe { load_symbol(&library, b"kitty_singbox_free_string\0")? };

        Ok(Self {
            _library: library,
            probe,
            probe_outbound,
            start,
            stop,
            version,
            last_error,
            traffic,
            logs,
            set_log_enabled,
            validate_rule_set_file,
            check_config,
            select_outbound,
            free_string,
        })
    }
}

unsafe fn load_symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, String> {
    unsafe { library.get::<T>(name) }
        .map(|symbol| *symbol)
        .map_err(|error| error.to_string())
}

fn bridge_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("KITTY_SINGBOX_DLL") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            candidates.push(parent.join("kitty_singbox.dll"));
        }
    }
    if let Some(path) = option_env!("KITTY_SINGBOX_BUILD_DLL") {
        candidates.push(PathBuf::from(path));
    }

    let mut unique = Vec::<OsString>::new();
    candidates.retain(|path| {
        let value = path.as_os_str().to_os_string();
        if unique.contains(&value) {
            false
        } else {
            unique.push(value);
            true
        }
    });
    candidates
}

pub(crate) fn bridge() -> Result<&'static Bridge, CoreError> {
    static BRIDGE: OnceLock<Result<Bridge, String>> = OnceLock::new();
    match BRIDGE.get_or_init(Bridge::load) {
        Ok(bridge) => Ok(bridge),
        Err(error) => Err(CoreError::BridgeUnavailable(error.clone())),
    }
}
