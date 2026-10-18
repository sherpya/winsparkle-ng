mod layout;
mod localization;
mod theme;

use crate::{
    appcast::{self, Release},
    crypto::PreparedFile,
    ffi,
    http::{CancellationToken, HttpClient, HttpRequest, WinHttp},
    platform::{self, TempDir, wide},
    settings,
    updater::{self, CheckMode, Config, InstallFlow, Phase},
};
use localization::Language;
use std::{
    path::PathBuf,
    ptr::null_mut,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::JoinHandle,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{Controls::*, HiDpi::*, Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
};

const WAKE: u32 = WM_APP + 17;
const DWLP_USER: i32 = (2 * std::mem::size_of::<isize>()) as i32;
pub struct Download {
    pub path: PathBuf,
    pub lock: PreparedFile,
    pub temp: TempDir,
}
pub enum Message {
    Check(CheckMode),
    Theme(updater::ThemeMode),
    Shutdown,
    Checked(u64, Box<crate::Result<Option<Release>>>),
    Progress(u64, u64, Option<u64>),
    Verifying(u64),
    Downloaded(u64, crate::Result<Download>),
}
#[derive(Clone)]
pub struct UiHandle {
    tx: Sender<Message>,
    hwnd: Arc<AtomicUsize>,
}
impl UiHandle {
    pub fn send(&self, message: Message) {
        if self.tx.send(message).is_err() {
            return;
        }
        let hwnd = self.hwnd.load(Ordering::Acquire);
        if hwnd != 0 {
            // SAFETY: PostMessage does not dereference the window; messages carry no raw payload.
            unsafe {
                PostMessageW(platform::raw(hwnd), WAKE, 0, 0);
            }
        }
    }
}
pub fn start(
    config: Config,
    shutdown: CancellationToken,
) -> crate::Result<(UiHandle, JoinHandle<()>)> {
    let (tx, rx) = mpsc::channel();
    let handle = UiHandle {
        tx,
        hwnd: Arc::new(AtomicUsize::new(0)),
    };
    let ui = handle.clone();
    let thread = std::thread::Builder::new()
        .name("WinSparkle UI".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(config, shutdown, ui, rx)
            }));
            match result {
                Ok(Err(e)) => platform::log(&e),
                Err(_) => platform::log("UI thread panic contained"),
                _ => {}
            }
            updater::ui_stopped();
        })
        .map_err(|e| e.to_string())?;
    Ok((handle, thread))
}

struct Ui {
    hwnd: HWND,
    config: Config,
    language: Language,
    handle: UiHandle,
    rx: Receiver<Message>,
    shutdown: CancellationToken,
    operation: CancellationToken,
    worker: Option<JoinHandle<()>>,
    generation: u64,
    phase: Phase,
    flow: InstallFlow,
    mode: CheckMode,
    release: Option<Release>,
    download: Option<Download>,
    visible: bool,
    error_occurred: bool,
    font: HFONT,
    heading_font: HFONT,
    theme: theme::Theme,
    theme_updating: bool,
    notes_borders: (isize, isize),
    quitting: bool,
    last_schedule: i64,
}
impl Drop for Ui {
    fn drop(&mut self) {
        self.operation.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.handle.hwnd.store(0, Ordering::Release);
        // SAFETY: these handles belong to this UI thread, and controls release fonts before deletion.
        unsafe {
            if !self.hwnd.is_null() {
                DestroyWindow(self.hwnd);
            }
            if !self.font.is_null() {
                DeleteObject(self.font);
            }
            if !self.heading_font.is_null() {
                DeleteObject(self.heading_font);
            }
        }
    }
}
fn run(
    config: Config,
    shutdown: CancellationToken,
    handle: UiHandle,
    rx: Receiver<Message>,
) -> crate::Result<()> {
    let language = Language::choose(&config);
    // SAFETY: DPI context is changed only on the new UI thread and restored before exit.
    let previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    struct Dpi(DPI_AWARENESS_CONTEXT);
    impl Drop for Dpi {
        fn drop(&mut self) {
            // SAFETY: restores the context returned by SetThreadDpiAwarenessContext on this thread.
            unsafe {
                if !self.0.is_null() {
                    SetThreadDpiAwarenessContext(self.0);
                }
            }
        }
    }
    let _dpi = Dpi(previous);
    let _activation = layout::Activation::new()?;
    let theme = theme::Theme::new(config.theme);
    let mut ui = Ui {
        hwnd: null_mut(),
        config,
        language,
        handle,
        rx,
        shutdown,
        operation: CancellationToken::default(),
        worker: None,
        generation: 0,
        phase: Phase::Idle,
        flow: InstallFlow::ConfirmAfterDownload,
        mode: CheckMode::Background,
        release: None,
        download: None,
        visible: false,
        error_occurred: false,
        font: null_mut(),
        heading_font: null_mut(),
        theme,
        theme_updating: false,
        notes_borders: (0, 0),
        quitting: false,
        last_schedule: 0,
    };
    // SAFETY: the Ui stack allocation stays at the same address until the dialog and loop are destroyed.
    unsafe {
        let controls = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS | ICC_STANDARD_CLASSES,
        };
        InitCommonControlsEx(&controls);
        let hwnd = CreateDialogParamW(
            platform::module(),
            101usize as *const u16,
            null_mut(),
            Some(dialog_proc),
            (&mut ui as *mut Ui) as isize,
        );
        if hwnd.is_null() {
            return Err(platform::error("Cannot create update dialog"));
        }
        ui.hwnd = hwnd;
        ui.handle.hwnd.store(hwnd as usize, Ordering::Release);
        layout::initialize(&mut ui);
        let notes = GetDlgItem(hwnd, 204);
        ui.notes_borders = (
            platform::window_long(notes, GWL_STYLE) & WS_BORDER as isize,
            platform::window_long(notes, GWL_EXSTYLE) & WS_EX_CLIENTEDGE as isize,
        );
        ui.refresh_theme();
        SetTimer(hwnd, 1, 1000, None);
    }
    ui.bootstrap();
    ui.drain();
    // SAFETY: standard message loop runs on the owning dialog thread; MSG is fully initialized.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        loop {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result <= 0 {
                break;
            }
            if IsDialogMessageW(ui.hwnd, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    Ok(())
}

unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: DWLP_USER holds the Ui pointer supplied at WM_INITDIALOG, only used on this UI thread.
        unsafe {
            if message == WM_INITDIALOG {
                platform::set_window_long(hwnd, DWLP_USER, lparam);
                return 1;
            }
            let ptr = platform::window_long(hwnd, DWLP_USER) as *mut Ui;
            if ptr.is_null() {
                return 0;
            }
            let ui = &*ptr;
            if message == WM_CTLCOLORDLG {
                return ui.theme.control_color(wparam as HDC, null_mut());
            }
            if message == WM_CTLCOLORSTATIC || message == WM_CTLCOLOREDIT {
                return ui.theme.control_color(wparam as HDC, lparam as HWND);
            }
            if message == WM_ERASEBKGND {
                ui.theme.erase(hwnd, wparam as HDC);
                return 1;
            }
            if [WM_SETTINGCHANGE, WM_THEMECHANGED, WM_SYSCOLORCHANGE].contains(&message) {
                (&mut *ptr).refresh_theme();
                return 0;
            }
            if ![WAKE, WM_COMMAND, WM_CLOSE, WM_TIMER].contains(&message) {
                return 0;
            }
            let ui = &mut *ptr;
            match message {
                WAKE => ui.drain(),
                WM_COMMAND => {
                    if (wparam >> 16) == BN_CLICKED as usize {
                        ui.command((wparam & 0xffff) as i32);
                    }
                }
                WM_CLOSE => ui.dismiss(),
                WM_TIMER => {
                    ui.drain();
                    if !ui.quitting {
                        ui.schedule();
                    }
                }
                _ => {}
            }
            1
        }
    }))
    .unwrap_or_else(|_| {
        platform::log("Panic caught in dialog procedure");
        0
    })
}

impl Ui {
    fn refresh_theme(&mut self) {
        if self.theme_updating || self.hwnd.is_null() {
            return;
        }
        self.theme_updating = true;
        self.theme = theme::Theme::new(self.config.theme);
        self.theme.apply(self.hwnd, self.notes_borders);
        self.theme_updating = false;
    }
    fn text(&self, id: u16) -> String {
        self.language.text(id)
    }
    fn set_text(&self, id: i32, text: &str) {
        // SAFETY: controls are owned by this thread; SetDlgItemTextW copies its UTF-16 input.
        unsafe {
            SetDlgItemTextW(self.hwnd, id, wide(text).as_ptr());
        }
    }
    fn show_control(&self, id: i32, show: bool) {
        // SAFETY: GetDlgItem returns a control owned by this live dialog.
        unsafe {
            ShowWindow(
                GetDlgItem(self.hwnd, id),
                if show { SW_SHOW } else { SW_HIDE },
            );
        }
    }
    fn set_progress_marquee(&self, marquee: bool) {
        // SAFETY: the progress control and its style are owned by this UI thread.
        unsafe {
            let bar = GetDlgItem(self.hwnd, 205);
            let style = platform::window_long(bar, GWL_STYLE);
            if (style & PBS_MARQUEE as isize != 0) == marquee {
                return;
            }
            // Changing mode restarts the themed animation, so only do it on transitions.
            if !marquee {
                SendMessageW(bar, PBM_SETMARQUEE, 0, 0);
            }
            platform::set_window_long(
                bar,
                GWL_STYLE,
                if marquee {
                    style | PBS_MARQUEE as isize
                } else {
                    style & !(PBS_MARQUEE as isize)
                },
            );
            if marquee {
                SendMessageW(bar, PBM_SETMARQUEE, 1, 40);
            }
        }
    }
    fn show(&mut self) {
        self.visible = true;
        // SAFETY: show/focus acts only on this UI thread's dialog.
        unsafe {
            ShowWindow(self.hwnd, SW_SHOWNORMAL);
            SetForegroundWindow(self.hwnd);
            SendDlgItemMessageW(self.hwnd, 204, EM_SETSEL, 0, 0);
            let primary = if [
                Phase::UpdateAvailable,
                Phase::ReadyToInstall,
                Phase::Permission,
            ]
            .contains(&self.phase)
            {
                207
            } else {
                2
            };
            SetFocus(GetDlgItem(self.hwnd, primary));
        }
    }
    fn bootstrap(&mut self) {
        if let Some(path) = settings::read(&self.config, "UpdateTempDir") {
            platform::clean_temp(std::path::Path::new(&path));
            settings::delete(&self.config, "UpdateTempDir");
        }
        match settings::read(&self.config, "CheckForUpdates") {
            Some(_) => self.schedule(),
            None if settings::number(&self.config, "DidRunOnce", 0) == 0 => {
                settings::write(&self.config, "DidRunOnce", "1")
            }
            None => {
                self.set_phase(Phase::Permission);
                self.show();
            }
        }
    }
    fn schedule(&mut self) {
        let now = updater::now();
        if self.last_schedule != 0 && now - self.last_schedule < 30 {
            return;
        }
        self.last_schedule = now;
        if self.phase != Phase::Idle || settings::number(&self.config, "CheckForUpdates", 0) == 0 {
            return;
        }
        let last = settings::number(&self.config, "LastCheckTime", -1);
        let interval = settings::number(&self.config, "UpdateInterval", 86400).max(3600);
        if last < 0 || now.saturating_sub(last) >= interval {
            self.begin_check(CheckMode::Background);
        }
    }
    fn drain(&mut self) {
        while let Ok(message) = self.rx.try_recv() {
            if self.quitting {
                break;
            }
            match message {
                Message::Shutdown => {
                    self.quitting = true;
                    self.operation.cancel();
                    // SAFETY: this is the owner thread of the updater message loop.
                    unsafe {
                        PostQuitMessage(0);
                    }
                }
                Message::Check(mode) => self.begin_check(mode),
                Message::Theme(theme) => {
                    self.config.theme = theme;
                    self.refresh_theme();
                }
                Message::Checked(id, result)
                    if id == self.generation && !self.operation.is_cancelled() =>
                {
                    self.join_worker();
                    match *result {
                        Ok(Some(release)) => {
                            settings::write(
                                &self.config,
                                "LastCheckTime",
                                &updater::now().to_string(),
                            );
                            self.release = Some(release);
                            self.set_phase(Phase::UpdateAvailable);
                            ffi::invoke(updater::snapshot().callbacks.found);
                            if self.shutdown.is_cancelled() {
                                continue;
                            }
                            self.show();
                            if self.flow == InstallFlow::InstallWithoutInitialPrompt {
                                self.begin_download();
                            }
                        }
                        Ok(None) => {
                            settings::write(
                                &self.config,
                                "LastCheckTime",
                                &updater::now().to_string(),
                            );
                            ffi::invoke(updater::snapshot().callbacks.not_found);
                            if self.mode == CheckMode::AutomaticInstall {
                                ffi::invoke(updater::snapshot().callbacks.cancelled);
                            }
                            self.set_phase(if self.mode == CheckMode::Background {
                                Phase::Idle
                            } else {
                                Phase::NoUpdate
                            });
                        }
                        Err(e) => self.fail(&e),
                    }
                }
                Message::Progress(id, received, total)
                    if id == self.generation && !self.operation.is_cancelled() =>
                {
                    let total = total.filter(|n| *n > 0);
                    let received_text = format_bytes(received);
                    self.set_text(
                        206,
                        &if let Some(total) = total {
                            self.language
                                .format(1023, &[&received_text, &format_bytes(total)])
                        } else {
                            received_text
                        },
                    );
                    self.set_progress_marquee(total.is_none());
                    if let Some(total) = total {
                        // SAFETY: sends progress updates only on the owner UI thread.
                        unsafe {
                            let bar = GetDlgItem(self.hwnd, 205);
                            SendMessageW(
                                bar,
                                PBM_SETPOS,
                                ((received as u128 * 1000 / total as u128).min(1000)) as usize,
                                0,
                            );
                        }
                    }
                }
                Message::Verifying(id)
                    if id == self.generation && !self.operation.is_cancelled() =>
                {
                    self.set_phase(Phase::Verifying)
                }
                Message::Downloaded(id, result)
                    if id == self.generation && !self.operation.is_cancelled() =>
                {
                    self.join_worker();
                    match result {
                        Ok(download) => {
                            settings::write(
                                &self.config,
                                "UpdateTempDir",
                                &download.temp.path.to_string_lossy(),
                            );
                            self.download = Some(download);
                            self.set_phase(Phase::ReadyToInstall);
                            if self.flow.install_after_download() {
                                self.install();
                            }
                        }
                        Err(e) => self.fail(&e),
                    }
                }
                _ => {}
            }
        }
    }
    fn join_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
    fn begin_check(&mut self, mode: CheckMode) {
        if self.shutdown.is_cancelled() {
            return;
        }
        if ![
            Phase::Idle,
            Phase::NoUpdate,
            Phase::Error,
            Phase::Cancelled,
            Phase::Permission,
        ]
        .contains(&self.phase)
        {
            if mode != CheckMode::Background {
                self.show();
            }
            return;
        }
        self.operation.cancel();
        self.join_worker();
        self.operation = CancellationToken::default();
        self.generation += 1;
        self.release = None;
        self.download = None;
        self.error_occurred = false;
        self.config.callbacks = updater::snapshot().callbacks;
        self.mode = mode;
        self.flow = InstallFlow::new(mode, self.config.one_click);
        self.set_phase(Phase::Checking);
        if mode != CheckMode::Background {
            self.show();
        }
        let config = self.config.clone();
        let token = self.operation.clone();
        let handle = self.handle.clone();
        let id = self.generation;
        let skip = if mode == CheckMode::Background {
            settings::read(&config, "SkipThisVersion")
        } else {
            None
        };
        let worker = std::thread::Builder::new()
            .name("WinSparkle check".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if config.feed.is_empty() {
                        return Err("Appcast URL is not configured".into());
                    }
                    let response = WinHttp.get_bytes(
                        HttpRequest {
                            url: &config.feed,
                            headers: &config.headers,
                            user_agent: &config.user_agent(),
                        },
                        &token,
                    )?;
                    let architecture = if cfg!(target_arch = "aarch64") {
                        "arm64"
                    } else if cfg!(target_arch = "x86") {
                        "x86"
                    } else {
                        "x64"
                    };
                    let releases = appcast::parse(&response.body, architecture)?;
                    let latest = releases
                        .into_iter()
                        .find(|r| platform::supports_os(&r.min_os));
                    Ok(latest.filter(|r| {
                        appcast::compare_versions(&r.version, config.build_version()).is_gt()
                            && (r.critical || skip.as_deref() != Some(&r.version))
                    }))
                }))
                .unwrap_or_else(|_| Err("Update check worker panic".into()));
                handle.send(Message::Checked(id, Box::new(result)));
            });
        match worker {
            Ok(worker) => self.worker = Some(worker),
            Err(e) => self.fail(&e.to_string()),
        }
    }
    fn begin_download(&mut self) {
        if self.shutdown.is_cancelled() {
            return;
        }
        let Some(release) = self.release.clone() else {
            return;
        };
        let Some(enclosure) = release.enclosure else {
            match crate::installer::open_url(&release.web_url) {
                Ok(()) => self.finish(),
                Err(e) => self.fail(&e),
            }
            return;
        };
        self.set_phase(Phase::Downloading);
        let config = self.config.clone();
        let token = self.operation.clone();
        let handle = self.handle.clone();
        let id = self.generation;
        let worker = std::thread::Builder::new()
            .name("WinSparkle download".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if config.dsa_required && config.key.is_none() {
                        return Err(
                            "Legacy DSA signatures are unsupported. Configure an Ed25519 key."
                                .into(),
                        );
                    }
                    let temp = TempDir::create()?;
                    let path = temp.path.join(installer_name(&enclosure.url));
                    WinHttp.download(
                        HttpRequest {
                            url: &enclosure.url,
                            headers: &config.headers,
                            user_agent: &config.user_agent(),
                        },
                        &path,
                        &token,
                        &|received, total| handle.send(Message::Progress(id, received, total)),
                    )?;
                    token.check()?;
                    let lock = PreparedFile::open(&path)?;
                    if let Some(key) = config.key.as_deref() {
                        handle.send(Message::Verifying(id));
                        lock.verify(key, &enclosure.signature)
                            .map_err(|e| format!("SIGNATURE: {e}"))?;
                    } else {
                        platform::log("Using unsigned updates: no trusted Ed25519 key configured");
                    }
                    token.check()?;
                    Ok(Download { path, lock, temp })
                }))
                .unwrap_or_else(|_| Err("Download worker panic".into()));
                handle.send(Message::Downloaded(id, result));
            });
        match worker {
            Ok(worker) => self.worker = Some(worker),
            Err(e) => self.fail(&e.to_string()),
        }
    }
    fn install(&mut self) {
        if self.shutdown.is_cancelled() || self.download.is_none() {
            return;
        }
        self.set_phase(Phase::PreparingInstall);
        let callbacks = updater::snapshot().callbacks;
        // SAFETY: host callback takes no arguments; invoked without locks on the dedicated UI thread.
        if callbacks.can_shutdown.is_some_and(|f| unsafe { f() } == 0) {
            self.set_phase(Phase::ReadyToInstall);
            self.set_text(202, &self.text(1021));
            self.set_text(207, &self.text(1011));
            return;
        }
        if self.shutdown.is_cancelled() {
            return;
        }
        self.set_phase(Phase::LaunchingInstaller);
        let download = self.download.as_mut().unwrap();
        let arguments = self
            .release
            .as_ref()
            .and_then(|r| r.enclosure.as_ref())
            .map(|e| e.arguments.as_str())
            .unwrap_or("");
        // The PreparedFile remains alive across callback/ShellExecute, denying write/delete races.
        let _keep_lock_alive = &download.lock;
        match crate::installer::launch(&download.path, arguments, callbacks.installer) {
            Ok(()) => {
                download.temp.keep = true;
                self.finish();
                ffi::invoke(callbacks.shutdown);
            }
            Err(e) => {
                platform::log(&e);
                self.set_phase(Phase::ReadyToInstall);
                self.set_text(202, &self.text(1033));
                self.set_text(207, &self.text(1011));
                ffi::invoke(callbacks.error);
            }
        }
    }
    fn fail(&mut self, error: &str) {
        if self.shutdown.is_cancelled() || self.operation.is_cancelled() {
            return;
        }
        self.error_occurred = true;
        platform::log(error);
        self.download = None;
        self.set_phase(Phase::Error);
        if error.starts_with("SIGNATURE:") {
            self.set_text(202, &self.text(1036));
        } else {
            self.set_text(202, error);
        }
        ffi::invoke(updater::snapshot().callbacks.error);
        if self.visible {
            self.show();
        } else {
            self.phase = Phase::Idle;
        }
    }
    fn command(&mut self, id: i32) {
        match id {
            2 => self.dismiss(),
            207 => match self.phase {
                Phase::Permission => {
                    settings::write(&self.config, "CheckForUpdates", "1");
                    self.phase = Phase::Idle;
                    self.dismiss();
                    self.last_schedule = 0;
                    self.schedule();
                }
                Phase::UpdateAvailable => self.begin_download(),
                Phase::ReadyToInstall => self.install(),
                _ => self.dismiss(),
            },
            208 => {
                ffi::invoke(updater::snapshot().callbacks.postponed);
                self.dismiss();
            }
            209 => {
                if let Some(release) = &self.release
                    && !release.critical
                {
                    settings::write(&self.config, "SkipThisVersion", &release.version);
                    ffi::invoke(updater::snapshot().callbacks.skipped);
                    self.dismiss();
                }
            }
            210 => {
                if let Some(release) = &self.release
                    && let Err(e) = crate::installer::open_url(&release.notes_url)
                {
                    platform::log(&e);
                }
            }
            _ => {}
        }
    }
    fn finish(&mut self) {
        self.set_phase(Phase::Completed);
        self.visible = false;
        // SAFETY: hide this thread's window; it stays alive for subsequent checks.
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
        }
        self.download = None;
        self.release = None;
        self.phase = Phase::Idle;
    }
    fn dismiss(&mut self) {
        self.operation.cancel();
        self.generation += 1;
        self.visible = false;
        // SAFETY: window belongs to this thread and is not destroyed until the loop exits.
        unsafe {
            ShowWindow(self.hwnd, SW_HIDE);
        }
        self.join_worker();
        self.download = None;
        let callbacks = updater::snapshot().callbacks;
        if self.release.is_some() && !self.error_occurred {
            ffi::invoke(callbacks.cancelled);
        }
        ffi::invoke(callbacks.dismissed);
        if self.phase == Phase::Permission {
            settings::write(&self.config, "CheckForUpdates", "0");
        }
        self.release = None;
        self.phase = Phase::Cancelled;
        self.phase = Phase::Idle;
    }
    fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
        for id in [203, 204, 205, 206, 207, 208, 209, 210, 2] {
            self.show_control(id, false);
        }
        let (heading, message, button) = match phase {
            Phase::Permission => (
                1027,
                self.language.format(1028, &[&self.config.app]),
                Some(1026),
            ),
            Phase::Checking => (1012, self.text(1035), None),
            Phase::NoUpdate => (
                1013,
                self.language
                    .format(1024, &[&self.config.app, &self.config.version]),
                None,
            ),
            Phase::UpdateAvailable => {
                let r = self.release.as_ref().unwrap();
                (
                    if r.critical { 1025 } else { 1001 },
                    self.language.format(
                        if self.flow.install_after_download() {
                            1019
                        } else {
                            1018
                        },
                        &[&self.config.app, r.display_version(), &self.config.version],
                    ),
                    Some(if r.enclosure.is_none() {
                        1016
                    } else if self.flow.install_after_download() {
                        1003
                    } else {
                        1002
                    }),
                )
            }
            Phase::Downloading => (1007, self.text(1030), None),
            Phase::Verifying => (1008, self.text(1031), None),
            Phase::ReadyToInstall => (1004, self.text(1020), Some(1004)),
            Phase::PreparingInstall | Phase::LaunchingInstaller => (1009, self.text(1032), None),
            Phase::Error => (1022, String::new(), None),
            _ => (1000, String::new(), None),
        };
        self.set_text(201, &self.text(heading));
        self.set_text(202, &message);
        self.set_text(206, "");
        if let Some(button) = button {
            self.set_text(207, &self.text(button));
            self.show_control(207, true);
        }
        if phase == Phase::UpdateAvailable {
            let release = self.release.as_ref().unwrap();
            self.set_text(203, &self.text(1015));
            let fallback = self.text(1037);
            self.set_text(
                204,
                if release.description.is_empty() {
                    &fallback
                } else {
                    &release.description
                },
            );
            self.show_control(203, true);
            self.show_control(204, true);
            self.set_text(208, &self.text(1005));
            self.set_text(209, &self.text(1006));
            self.show_control(208, true);
            self.show_control(209, !release.critical);
            if !release.notes_url.is_empty() {
                self.set_text(210, &self.text(1017));
                self.show_control(210, true);
            }
        } else {
            self.set_text(
                2,
                &self.text(if phase == Phase::Permission {
                    1029
                } else if [Phase::Checking, Phase::Downloading, Phase::Verifying].contains(&phase) {
                    1010
                } else {
                    1014
                }),
            );
            self.show_control(
                2,
                ![Phase::LaunchingInstaller, Phase::PreparingInstall].contains(&phase),
            );
        }
        if [Phase::Checking, Phase::Downloading, Phase::Verifying].contains(&phase) {
            self.set_progress_marquee(phase != Phase::Downloading);
            if phase == Phase::Downloading {
                // Reset while hidden so a new download never exposes the previous animation or position.
                // SAFETY: this control belongs to the current UI thread and is in determinate mode.
                unsafe {
                    SendDlgItemMessageW(self.hwnd, 205, PBM_SETPOS, 0, 0);
                }
            }
            self.show_control(205, true);
            self.show_control(206, true);
        }
        layout::arrange(self);
        // SAFETY: default button/focus are applied to visible controls of this dialog.
        unsafe {
            let primary = if button.is_some() { 207 } else { 2 };
            SendMessageW(self.hwnd, DM_SETDEFID, primary as usize, 0);
            if self.visible {
                SetFocus(GetDlgItem(self.hwnd, primary));
            }
        }
    }
}
fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KiB", bytes as f64 / 1024.0)
    }
}
fn installer_name(url: &str) -> String {
    let name = url
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('/')
        .next()
        .unwrap_or("");
    let safe: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || ['.', '-', '_'].contains(c))
        .take(120)
        .collect();
    if safe.contains('.') && !safe.starts_with('.') {
        format!("update-{safe}")
    } else {
        "update.exe".into()
    }
}
