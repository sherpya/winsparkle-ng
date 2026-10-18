use super::Ui;
use crate::platform::{self, wide};
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{ApplicationInstallationAndServicing::*, WindowsProgramming::*},
    UI::{Controls::*, WindowsAndMessaging::*},
};

pub struct Activation {
    handle: HANDLE,
    cookie: usize,
}
impl Activation {
    pub fn new() -> crate::Result<Self> {
        // SAFETY: activation context references manifest resource 2 in this loaded DLL.
        unsafe {
            let mut context: ACTCTXW = std::mem::zeroed();
            context.cbSize = std::mem::size_of_val(&context) as u32;
            context.dwFlags = ACTCTX_FLAG_HMODULE_VALID | ACTCTX_FLAG_RESOURCE_NAME_VALID;
            context.hModule = platform::module();
            context.lpResourceName = std::ptr::dangling::<u16>();
            let handle = CreateActCtxW(&context);
            if handle == INVALID_HANDLE_VALUE {
                return Err(platform::error("Common controls activation context"));
            }
            let mut cookie = 0;
            if ActivateActCtx(handle, &mut cookie) == 0 {
                ReleaseActCtx(handle);
                return Err(platform::error("Activate common controls"));
            }
            Ok(Self { handle, cookie })
        }
    }
}
impl Drop for Activation {
    fn drop(&mut self) {
        // SAFETY: this context was activated on this same UI thread and is deactivated exactly once.
        unsafe {
            DeactivateActCtx(0, self.cookie);
            ReleaseActCtx(self.handle);
        }
    }
}
pub fn initialize(ui: &mut Ui) {
    // SAFETY: all controls and fonts are initialized/used on the window's owner thread.
    unsafe {
        SetWindowTextW(ui.hwnd, wide(&ui.text(1000)).as_ptr());
        if ui.language.rtl {
            platform::set_window_long(
                ui.hwnd,
                GWL_EXSTYLE,
                platform::window_long(ui.hwnd, GWL_EXSTYLE) | WS_EX_LAYOUTRTL as isize,
            );
        }
        let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
        metrics.cbSize = std::mem::size_of_val(&metrics) as u32;
        if SystemParametersInfoW(
            SPI_GETNONCLIENTMETRICS,
            metrics.cbSize,
            (&mut metrics as *mut NONCLIENTMETRICSW).cast(),
            0,
        ) != 0
        {
            ui.font = CreateFontIndirectW(&metrics.lfMessageFont);
            metrics.lfMessageFont.lfHeight = metrics.lfMessageFont.lfHeight * 5 / 4;
            metrics.lfMessageFont.lfWeight = 600;
            ui.heading_font = CreateFontIndirectW(&metrics.lfMessageFont);
            for id in [201, 202, 203, 204, 206, 207, 208, 209, 210, 2] {
                SendDlgItemMessageW(ui.hwnd, id, WM_SETFONT, ui.font as usize, 1);
            }
            SendDlgItemMessageW(ui.hwnd, 201, WM_SETFONT, ui.heading_font as usize, 1);
        }
        let icon = LoadImageW(
            platform::module(),
            102usize as *const u16,
            IMAGE_ICON,
            48,
            48,
            LR_SHARED,
        );
        SendDlgItemMessageW(ui.hwnd, 200, STM_SETICON, icon as usize, 0);
        SendMessageW(ui.hwnd, WM_SETICON, ICON_BIG as usize, icon as isize);
        SendMessageW(ui.hwnd, WM_SETICON, ICON_SMALL as usize, icon as isize);
        SendDlgItemMessageW(ui.hwnd, 204, EM_SETLIMITTEXT, 1024 * 1024, 0);
        SendDlgItemMessageW(ui.hwnd, 205, PBM_SETRANGE32, 0, 1000);
    }
}
pub fn arrange(ui: &Ui) {
    use crate::updater::Phase;
    // SAFETY: layout exclusively manipulates controls on their owning UI thread, with system metrics.
    unsafe {
        let mut units = RECT {
            left: 0,
            top: 0,
            right: 4,
            bottom: 8,
        };
        MapDialogRect(ui.hwnd, &mut units);
        let scale = units.bottom as f32 / 16.0;
        let px = |v: i32| (v as f32 * scale).round() as i32;
        let dc = GetDC(ui.hwnd);
        let old = SelectObject(dc, ui.font);
        let visible = |id| {
            platform::window_long(GetDlgItem(ui.hwnd, id), GWL_STYLE) & WS_VISIBLE as isize != 0
        };
        let button_width = |id: i32| {
            let mut text = vec![0u16; 1024];
            let len = GetDlgItemTextW(ui.hwnd, id, text.as_mut_ptr(), text.len() as i32);
            let mut size: SIZE = std::mem::zeroed();
            GetTextExtentPoint32W(dc, text.as_ptr(), len as i32, &mut size);
            (size.cx + px(30)).max(px(90))
        };
        let mut buttons = Vec::new();
        for id in [209, 208, 2, 207] {
            if visible(id) {
                buttons.push((id, button_width(id)));
            }
        }
        let margin = px(22);
        let gap = px(10);
        let width = px(620).max(
            buttons.iter().map(|(_, w)| *w).sum::<i32>()
                + gap * (buttons.len() as i32 - 1).max(0)
                + margin * 2,
        );
        let available = ui.phase == Phase::UpdateAvailable;
        let body_x = px(88);
        let text_width = width - body_x - margin;
        let mut message = vec![0u16; 8192];
        let len = GetDlgItemTextW(ui.hwnd, 202, message.as_mut_ptr(), message.len() as i32);
        let mut body = RECT {
            left: 0,
            top: 0,
            right: text_width,
            bottom: 0,
        };
        DrawTextW(
            dc,
            message.as_ptr(),
            len as i32,
            &mut body,
            DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX,
        );
        let message_height = body.bottom.max(px(42));
        let content_bottom = px(74) + message_height;
        let height = if available {
            content_bottom + px(264)
        } else {
            content_bottom + px(132)
        };
        SelectObject(dc, old);
        ReleaseDC(ui.hwnd, dc);
        let mut outer = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        AdjustWindowRectEx(
            &mut outer,
            platform::window_long(ui.hwnd, GWL_STYLE) as u32,
            0,
            platform::window_long(ui.hwnd, GWL_EXSTYLE) as u32,
        );
        let mut work: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work as *mut RECT).cast(), 0);
        let ow = outer.right - outer.left;
        let oh = outer.bottom - outer.top;
        let mut current: RECT = std::mem::zeroed();
        GetWindowRect(ui.hwnd, &mut current);
        let (x, y) = if ui.visible {
            (
                current.left.min(work.right - ow).max(work.left),
                current.top.min(work.bottom - oh).max(work.top),
            )
        } else {
            (
                work.left + (work.right - work.left - ow) / 2,
                work.top + (work.bottom - work.top - oh) / 2,
            )
        };
        SetWindowPos(
            ui.hwnd,
            null_mut(),
            x,
            y,
            ow,
            oh,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let place = |id, x, y, w, h| {
            SetWindowPos(
                GetDlgItem(ui.hwnd, id),
                null_mut(),
                x,
                y,
                w,
                h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        };
        place(200, margin, px(24), px(48), px(48));
        place(201, body_x, px(22), text_width, px(42));
        place(202, body_x, px(74), text_width, message_height);
        place(
            203,
            margin,
            content_bottom + px(16),
            width - 2 * margin,
            px(20),
        );
        place(
            204,
            margin,
            content_bottom + px(42),
            width - 2 * margin,
            px(118),
        );
        place(
            210,
            margin,
            content_bottom + px(164),
            width - 2 * margin,
            px(27),
        );
        place(205, body_x, content_bottom + px(10), text_width, px(14));
        place(206, body_x, content_bottom + px(32), text_width, px(28));
        place(211, 0, height - px(58), width, px(2));
        let mut right = width - margin;
        for &(id, w) in buttons.iter().rev() {
            let x = if id == 209 { margin } else { right - w };
            place(id, x, height - px(42), w, px(29));
            right = x - gap;
        }
        InvalidateRect(ui.hwnd, null(), 1);
    }
}
