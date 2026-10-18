use crate::{platform::wide, updater::ThemeMode};
use std::{mem::size_of, ptr::null};
use windows_sys::Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::{Dwm::*, Gdi::*},
    System::{Registry::*, SystemInformation::GetTickCount64},
    UI::{
        Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
        Controls::*,
        Input::KeyboardAndMouse::IsWindowEnabled,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const THEME_SUBCLASS: usize = 0x574e47;
const MARQUEE_TIMER: usize = 0x574e4701;

struct Brush(HBRUSH, bool);
impl Brush {
    fn solid(color: COLORREF, fallback: SYS_COLOR_INDEX) -> Self {
        // SAFETY: solid brushes have no borrowed storage; the fallback is a borrowed system brush.
        unsafe {
            let brush = CreateSolidBrush(color);
            if brush.is_null() {
                Self(GetSysColorBrush(fallback), false)
            } else {
                Self(brush, true)
            }
        }
    }
}
impl Drop for Brush {
    fn drop(&mut self) {
        if self.1 {
            // SAFETY: this brush is owned, no longer selected, and outlives the dialog's painting.
            unsafe { DeleteObject(self.0) };
        }
    }
}

pub struct Theme {
    pub dark: bool,
    pub background: COLORREF,
    pub foreground: COLORREF,
    pub edit: COLORREF,
    background_brush: Brush,
    footer_brush: Brush,
    edit_brush: Brush,
}

fn use_dark(mode: ThemeMode, system_dark: bool, high_contrast: bool) -> bool {
    !high_contrast
        && match mode {
            ThemeMode::Auto => system_dark,
            ThemeMode::Light => false,
            ThemeMode::Dark => true,
        }
}

pub fn high_contrast() -> bool {
    // SAFETY: HIGHCONTRASTW is initialized with its required size and valid writable storage.
    unsafe {
        let mut contrast: HIGHCONTRASTW = std::mem::zeroed();
        contrast.cbSize = size_of::<HIGHCONTRASTW>() as u32;
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            (&mut contrast as *mut HIGHCONTRASTW).cast(),
            0,
        ) != 0
            && contrast.dwFlags & HCF_HIGHCONTRASTON != 0
    }
}

fn system_dark() -> bool {
    // SAFETY: RegGetValue reads only the current user's application color preference.
    unsafe {
        let mut light = 1u32;
        let mut length = size_of::<u32>() as u32;
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            wide("AppsUseLightTheme").as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut light as *mut u32).cast(),
            &mut length,
        ) == 0
            && light == 0
    }
}

impl Theme {
    pub fn new(mode: ThemeMode) -> Self {
        let dark = use_dark(mode, system_dark(), high_contrast());
        // SAFETY: system colors are available for all supported Windows versions and high contrast.
        let (background, foreground, footer, edit) = unsafe {
            if dark {
                (0x202020, 0xf0f0f0, 0x282828, 0x181818)
            } else {
                (
                    GetSysColor(COLOR_WINDOW),
                    GetSysColor(COLOR_WINDOWTEXT),
                    GetSysColor(COLOR_BTNFACE),
                    GetSysColor(COLOR_WINDOW),
                )
            }
        };
        Self {
            dark,
            background,
            foreground,
            edit,
            background_brush: Brush::solid(background, COLOR_WINDOW),
            footer_brush: Brush::solid(footer, COLOR_BTNFACE),
            edit_brush: Brush::solid(edit, COLOR_WINDOW),
        }
    }

    pub fn apply(&self, hwnd: HWND, notes_borders: (isize, isize)) {
        // SAFETY: every HWND belongs to this updater UI thread. No process-wide theme API is used.
        unsafe {
            let dark = i32::from(self.dark);
            // Older Windows builds can reject these attributes; client painting remains available.
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                (&dark as *const i32).cast(),
                size_of::<i32>() as u32,
            );
            for (attribute, color) in [
                (
                    DWMWA_CAPTION_COLOR,
                    if self.dark {
                        self.background
                    } else {
                        DWMWA_COLOR_DEFAULT
                    },
                ),
                (
                    DWMWA_TEXT_COLOR,
                    if self.dark {
                        self.foreground
                    } else {
                        DWMWA_COLOR_DEFAULT
                    },
                ),
            ] {
                DwmSetWindowAttribute(
                    hwnd,
                    attribute as u32,
                    (&color as *const u32).cast(),
                    size_of::<u32>() as u32,
                );
            }
            let explorer = wide("DarkMode_Explorer");
            // This optional theme class improves native scrollbars where available; older versions
            // retain native scrollbars. Text/background painting does not depend on the class.
            SetWindowTheme(
                GetDlgItem(hwnd, 204),
                if self.dark { explorer.as_ptr() } else { null() },
                null(),
            );
            let notes = GetDlgItem(hwnd, 204);
            let style = crate::platform::window_long(notes, GWL_STYLE);
            crate::platform::set_window_long(
                notes,
                GWL_STYLE,
                if self.dark {
                    style & !(WS_BORDER as isize)
                } else {
                    (style & !(WS_BORDER as isize)) | notes_borders.0
                },
            );
            let extended =
                crate::platform::window_long(notes, GWL_EXSTYLE) & !(WS_EX_CLIENTEDGE as isize);
            crate::platform::set_window_long(
                notes,
                GWL_EXSTYLE,
                extended | if self.dark { 0 } else { notes_borders.1 },
            );
            SetWindowPos(
                notes,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            // Ui remains at a stable address until its windows are destroyed. Replacing its
            // Theme field preserves this address, and subclasses are removed at WM_NCDESTROY.
            for id in [205, 2, 207, 208, 209, 210] {
                let control = GetDlgItem(hwnd, id);
                SetWindowSubclass(
                    control,
                    Some(control_proc),
                    THEME_SUBCLASS,
                    self as *const Self as usize,
                );
                if id == 205 {
                    if self.dark
                        && crate::platform::window_long(control, GWL_STYLE) & PBS_MARQUEE as isize
                            != 0
                    {
                        SetTimer(control, MARQUEE_TIMER, 40, None);
                    } else {
                        KillTimer(control, MARQUEE_TIMER);
                    }
                }
            }
            ShowWindow(
                GetDlgItem(hwnd, 211),
                if self.dark { SW_HIDE } else { SW_SHOW },
            );
            RedrawWindow(
                hwnd,
                null(),
                std::ptr::null_mut(),
                RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_ALLCHILDREN,
            );
        }
    }

    pub fn control_color(&self, dc: HDC, control: HWND) -> isize {
        // SAFETY: Windows supplies the live control's painting DC for WM_CTLCOLOR messages.
        unsafe {
            let notes = !control.is_null() && GetDlgCtrlID(control) == 204;
            SetTextColor(dc, self.foreground);
            SetBkColor(dc, if notes { self.edit } else { self.background });
            (if notes {
                self.edit_brush.0
            } else {
                self.background_brush.0
            }) as isize
        }
    }

    pub fn erase(&self, hwnd: HWND, dc: HDC) {
        // SAFETY: rectangles and brushes are valid for this dialog's painting DC.
        unsafe {
            let mut rect: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rect);
            FillRect(dc, &rect, self.background_brush.0);
            let notes = GetDlgItem(hwnd, 204);
            if self.dark && IsWindowVisible(notes) != 0 {
                let mut border: RECT = std::mem::zeroed();
                GetWindowRect(notes, &mut border);
                MapWindowPoints(
                    std::ptr::null_mut(),
                    hwnd,
                    (&mut border as *mut RECT).cast(),
                    2,
                );
                InflateRect(&mut border, 1, 1);
                let line = Brush::solid(0x686868, COLOR_BTNSHADOW);
                FrameRect(dc, &border, line.0);
            }
            let separator = GetDlgItem(hwnd, 211);
            if !separator.is_null() {
                let mut boundary: RECT = std::mem::zeroed();
                GetWindowRect(separator, &mut boundary);
                MapWindowPoints(
                    std::ptr::null_mut(),
                    hwnd,
                    (&mut boundary as *mut RECT).cast(),
                    2,
                );
                rect.top = boundary.top;
                FillRect(dc, &rect, self.footer_brush.0);
                if self.dark {
                    rect.bottom = rect.top + 1;
                    let line = Brush::solid(0x404040, COLOR_BTNSHADOW);
                    FillRect(dc, &rect, line.0);
                }
            }
        }
    }

    fn paint_button(&self, hwnd: HWND, dc: HDC) {
        // SAFETY: the subclass supplies this button's live painting DC; native input/accessibility remain intact.
        unsafe {
            let saved = SaveDC(dc);
            let state = SendMessageW(hwnd, BM_GETSTATE, 0, 0) as u32;
            let mut bounds: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut bounds);
            FillRect(
                dc,
                &bounds,
                if GetDlgCtrlID(hwnd) == 210 {
                    self.background_brush.0
                } else {
                    self.footer_brush.0
                },
            );
            let primary = crate::platform::window_long(hwnd, GWL_STYLE) & BS_TYPEMASK as isize
                == BS_DEFPUSHBUTTON as isize;
            let background = if state & BST_PUSHED != 0 {
                0x292929
            } else if state & BST_HOT != 0 {
                0x484848
            } else {
                0x363636
            };
            let border = if primary || state & BST_FOCUS != 0 {
                0xe5b86a
            } else {
                0x686868
            };
            let fill = Brush::solid(background, COLOR_BTNFACE);
            let pen = CreatePen(PS_SOLID, 1, border);
            let old_pen = SelectObject(dc, pen);
            let old_brush = SelectObject(dc, fill.0);
            RoundRect(
                dc,
                bounds.left,
                bounds.top,
                bounds.right,
                bounds.bottom,
                6,
                6,
            );
            SelectObject(dc, old_brush);
            SelectObject(dc, old_pen);
            DeleteObject(pen);
            SelectObject(dc, SendMessageW(hwnd, WM_GETFONT, 0, 0) as HFONT);
            SetBkMode(dc, TRANSPARENT as i32);
            SetTextColor(
                dc,
                if IsWindowEnabled(hwnd) == 0 {
                    0x909090
                } else {
                    self.foreground
                },
            );
            let mut text = vec![0u16; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
            let count = GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32);
            let mut rect = bounds;
            let mut flags = DT_CENTER | DT_VCENTER | DT_SINGLELINE;
            if SendMessageW(hwnd, WM_QUERYUISTATE, 0, 0) as u32 & UISF_HIDEACCEL != 0 {
                flags |= DT_HIDEPREFIX;
            }
            DrawTextW(dc, text.as_ptr(), count, &mut rect, flags);
            if state & BST_FOCUS != 0
                && SendMessageW(hwnd, WM_QUERYUISTATE, 0, 0) as u32 & UISF_HIDEFOCUS == 0
            {
                InflateRect(&mut rect, -4, -4);
                DrawFocusRect(dc, &rect);
            }
            RestoreDC(dc, saved);
        }
    }

    fn paint_progress(&self, hwnd: HWND, dc: HDC) {
        // SAFETY: all progress state is queried on the control's owner thread and rectangles are bounded.
        unsafe {
            let mut rect: RECT = std::mem::zeroed();
            GetClientRect(hwnd, &mut rect);
            FillRect(dc, &rect, self.edit_brush.0);
            let border = Brush::solid(0x686868, COLOR_BTNSHADOW);
            FrameRect(dc, &rect, border.0);
            InflateRect(&mut rect, -2, -2);
            let width = (rect.right - rect.left).max(0);
            if crate::platform::window_long(hwnd, GWL_STYLE) & PBS_MARQUEE as isize != 0 {
                let block = (width / 4).max(1);
                let offset = ((GetTickCount64() % 1600) * (width + block) as u64 / 1600) as i32;
                let left = rect.left;
                rect.left = (left + offset - block).max(left);
                rect.right = (left + offset).min(rect.right);
            } else {
                let position = SendMessageW(hwnd, PBM_GETPOS, 0, 0).clamp(0, 1000);
                rect.right = rect.left + (width as isize * position / 1000) as i32;
            }
            if rect.right > rect.left {
                let fill = Brush::solid(0xe5b86a, COLOR_HIGHLIGHT);
                FillRect(dc, &rect, fill.0);
            }
        }
    }
}

unsafe extern "system" fn control_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // SAFETY: data points to the stable Theme field until the owning Ui destroys this child.
        unsafe {
            if message == WM_NCDESTROY {
                KillTimer(hwnd, MARQUEE_TIMER);
                RemoveWindowSubclass(hwnd, Some(control_proc), THEME_SUBCLASS);
                return DefSubclassProc(hwnd, message, wparam, lparam);
            }
            let theme = &*(data as *const Theme);
            if !theme.dark {
                return DefSubclassProc(hwnd, message, wparam, lparam);
            }
            let progress = GetDlgCtrlID(hwnd) == 205;
            if message == WM_ERASEBKGND {
                return 1;
            }
            if message == WM_PAINT || message == WM_PRINTCLIENT {
                let mut paint: PAINTSTRUCT = std::mem::zeroed();
                let dc = if message == WM_PAINT {
                    BeginPaint(hwnd, &mut paint)
                } else {
                    wparam as HDC
                };
                if progress {
                    theme.paint_progress(hwnd, dc)
                } else {
                    theme.paint_button(hwnd, dc)
                }
                if message == WM_PAINT {
                    EndPaint(hwnd, &paint);
                }
                return 0;
            }
            if progress && message == WM_TIMER {
                if wparam == MARQUEE_TIMER && IsWindowVisible(hwnd) != 0 {
                    InvalidateRect(hwnd, null(), 0);
                }
                return 0;
            }
            let result = DefSubclassProc(hwnd, message, wparam, lparam);
            if progress && message == PBM_SETMARQUEE {
                if wparam != 0 {
                    SetTimer(hwnd, MARQUEE_TIMER, 40, None);
                } else {
                    KillTimer(hwnd, MARQUEE_TIMER);
                }
            }
            if [
                PBM_SETPOS,
                PBM_SETMARQUEE,
                BM_SETSTATE,
                BM_SETSTYLE,
                WM_SETTEXT,
                WM_SETFOCUS,
                WM_KILLFOCUS,
                WM_UPDATEUISTATE,
                WM_MOUSEMOVE,
                WM_MOUSELEAVE,
            ]
            .contains(&message)
            {
                InvalidateRect(hwnd, null(), 0);
            }
            result
        }
    }))
    .unwrap_or_else(|_| {
        crate::platform::log("Panic caught in themed control procedure");
        // SAFETY: preserve the native control's behavior if custom painting fails.
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preference_and_accessibility_precedence() {
        for mode in [ThemeMode::Auto, ThemeMode::Light, ThemeMode::Dark] {
            assert!(!use_dark(mode, true, true));
            assert!(!use_dark(mode, false, true));
        }
        assert!(use_dark(ThemeMode::Auto, true, false));
        assert!(!use_dark(ThemeMode::Auto, false, false));
        assert!(use_dark(ThemeMode::Dark, false, false));
        assert!(!use_dark(ThemeMode::Light, true, false));
        assert_eq!(ThemeMode::from_abi(-1), None);
        assert_eq!(ThemeMode::from_abi(3), None);
    }
}
