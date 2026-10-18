use crate::{
    ffi::{Callbacks, ConfigMethods},
    http::CancellationToken,
    ui::{Message, UiHandle},
};
use std::{
    sync::{Condvar, Mutex, OnceLock},
    thread::{JoinHandle, ThreadId},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct Config {
    pub initialized: bool,
    pub company: String,
    pub app: String,
    pub version: String,
    pub build: String,
    pub feed: String,
    pub key: Option<String>,
    pub dsa_required: bool,
    pub lang: Option<String>,
    pub langid: Option<u16>,
    pub registry: String,
    pub headers: Vec<(String, String)>,
    pub one_click: bool,
    pub theme: ThemeMode,
    pub callbacks: Callbacks,
    pub methods: Option<ConfigMethods>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            initialized: false,
            company: String::new(),
            app: String::new(),
            version: String::new(),
            build: String::new(),
            feed: String::new(),
            key: None,
            dsa_required: false,
            lang: None,
            langid: None,
            registry: String::new(),
            headers: Vec::new(),
            one_click: true,
            theme: ThemeMode::default(),
            callbacks: Callbacks::default(),
            methods: None,
        }
    }
}
impl Config {
    pub fn registry_path(&self) -> String {
        if !self.registry.is_empty() {
            return self.registry.clone();
        }
        if self.company.is_empty() {
            format!("Software\\{}\\WinSparkle", self.app)
        } else {
            format!("Software\\{}\\{}\\WinSparkle", self.company, self.app)
        }
    }
    pub fn user_agent(&self) -> String {
        format!(
            "{}/{} WinSparkle-NG/{}",
            self.app,
            self.version,
            env!("CARGO_PKG_VERSION")
        )
    }
    pub fn build_version(&self) -> &str {
        if self.build.is_empty() {
            &self.version
        } else {
            &self.build
        }
    }
    fn resolve(&mut self) {
        if self.feed.is_empty() {
            self.feed = crate::platform::host_resource("FeedURL", "APPCAST").unwrap_or_default();
        }
        if self.key.is_none() {
            self.key = crate::platform::host_resource("EdDSAPub", "EDDSA");
        }
        if self.key.is_none() && crate::platform::host_resource("DSAPub", "DSAPEM").is_some() {
            self.dsa_required = true;
        }
        let metadata = crate::platform::host_metadata();
        if self.company.is_empty() {
            self.company = metadata.0;
        }
        if self.app.is_empty() {
            self.app = metadata.1;
        }
        if self.version.is_empty() {
            self.version = metadata.2;
        }
        if self.app.is_empty() {
            self.app = "Application".into();
        }
    }
}
static CONFIG: OnceLock<Mutex<Config>> = OnceLock::new();
fn config() -> &'static Mutex<Config> {
    CONFIG.get_or_init(|| Mutex::new(Config::default()))
}
pub fn snapshot() -> Config {
    config().lock().unwrap_or_else(|p| p.into_inner()).clone()
}
pub fn configure(f: impl FnOnce(&mut Config)) {
    let mut c = config().lock().unwrap_or_else(|p| p.into_inner());
    if !c.initialized {
        f(&mut c);
    }
}
pub fn callbacks(f: impl FnOnce(&mut Callbacks)) {
    f(&mut config().lock().unwrap_or_else(|p| p.into_inner()).callbacks);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum ThemeMode {
    #[default]
    Auto = 0,
    Light = 1,
    Dark = 2,
}
impl ThemeMode {
    pub fn from_abi(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Auto),
            1 => Some(Self::Light),
            2 => Some(Self::Dark),
            _ => None,
        }
    }
}

pub fn set_theme(theme: ThemeMode) {
    let ui = {
        // Match init's runtime -> config lock order; never send while holding either lock.
        let slot = runtime().0.lock().unwrap_or_else(|p| p.into_inner());
        config().lock().unwrap_or_else(|p| p.into_inner()).theme = theme;
        slot.runtime.as_ref().map(|r| r.ui.clone())
    };
    if let Some(ui) = ui {
        ui.send(Message::Theme(theme));
    }
}

struct Runtime {
    ui: UiHandle,
    token: CancellationToken,
    thread: JoinHandle<()>,
}
#[derive(Default)]
struct RuntimeSlot {
    runtime: Option<Runtime>,
    closing: Option<ThreadId>,
}
static RUNTIME: OnceLock<(Mutex<RuntimeSlot>, Condvar)> = OnceLock::new();
fn runtime() -> &'static (Mutex<RuntimeSlot>, Condvar) {
    RUNTIME.get_or_init(|| (Mutex::new(RuntimeSlot::default()), Condvar::new()))
}
pub fn init() {
    let mut slot = runtime().0.lock().unwrap_or_else(|p| p.into_inner());
    if slot.runtime.is_some() || slot.closing.is_some() {
        return;
    }
    let cfg = {
        let mut cfg = config().lock().unwrap_or_else(|p| p.into_inner());
        cfg.resolve();
        cfg.initialized = true;
        cfg.clone()
    };
    let token = CancellationToken::default();
    match crate::ui::start(cfg, token.clone()) {
        Ok((ui, thread)) => slot.runtime = Some(Runtime { ui, token, thread }),
        Err(e) => {
            crate::platform::log(&e);
            config()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .initialized = false;
        }
    }
}
pub fn check(mode: CheckMode) {
    init();
    let ui = runtime()
        .0
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .runtime
        .as_ref()
        .map(|r| r.ui.clone());
    if let Some(ui) = ui {
        ui.send(Message::Check(mode));
    }
}
pub fn cleanup() {
    // Do not hold any global lock across joins or host callbacks.
    let current = std::thread::current().id();
    let runtime = {
        let (mutex, changed) = runtime();
        let mut slot = mutex.lock().unwrap_or_else(|p| p.into_inner());
        while let Some(closing) = slot.closing {
            if closing == current {
                return;
            }
            slot = changed.wait(slot).unwrap_or_else(|p| p.into_inner());
        }
        let instance = slot.runtime.take();
        if let Some(instance) = &instance {
            slot.closing = Some(instance.thread.thread().id());
        }
        instance
    };
    if let Some(runtime) = runtime {
        runtime.token.cancel();
        runtime.ui.send(Message::Shutdown);
        if runtime.thread.thread().id() != std::thread::current().id() {
            let _ = runtime.thread.join();
        }
    }
}
pub fn ui_stopped() {
    let (mutex, changed) = runtime();
    let mut slot = mutex.lock().unwrap_or_else(|p| p.into_inner());
    if slot
        .runtime
        .as_ref()
        .is_some_and(|r| r.thread.thread().id() == std::thread::current().id())
    {
        slot.runtime.take();
    }
    config()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .initialized = false;
    slot.closing = None;
    changed.notify_all();
}
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckMode {
    Background,
    Manual,
    AutomaticInstall,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallFlow {
    ConfirmAfterDownload,
    InstallAfterDownload,
    InstallWithoutInitialPrompt,
}
impl InstallFlow {
    pub fn new(mode: CheckMode, one_click: bool) -> Self {
        if mode == CheckMode::AutomaticInstall {
            Self::InstallWithoutInitialPrompt
        } else if one_click {
            Self::InstallAfterDownload
        } else {
            Self::ConfirmAfterDownload
        }
    }
    pub fn install_after_download(self) -> bool {
        self != Self::ConfirmAfterDownload
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Permission,
    Checking,
    NoUpdate,
    UpdateAvailable,
    Downloading,
    Verifying,
    ReadyToInstall,
    PreparingInstall,
    LaunchingInstaller,
    Completed,
    Cancelled,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn install_policy_defaults_to_one_click_and_preserves_opt_out() {
        let config = Config::default();
        for mode in [CheckMode::Manual, CheckMode::Background] {
            assert_eq!(
                InstallFlow::new(mode, config.one_click),
                InstallFlow::InstallAfterDownload
            );
        }
        assert_eq!(
            InstallFlow::new(CheckMode::Manual, false),
            InstallFlow::ConfirmAfterDownload
        );
        assert_eq!(
            InstallFlow::new(CheckMode::Background, true),
            InstallFlow::InstallAfterDownload
        );
        for flag in [true, false] {
            assert_eq!(
                InstallFlow::new(CheckMode::AutomaticInstall, flag),
                InstallFlow::InstallWithoutInitialPrompt
            );
        }
    }
}
