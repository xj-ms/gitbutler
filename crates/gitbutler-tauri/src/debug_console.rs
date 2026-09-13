use std::{
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use windows::{
    Win32::{
        Foundation::{HWND, LPARAM},
        System::Console::{GetConsoleProcessList, SetConsoleTitleW},
        UI::WindowsAndMessaging::{
            EnumWindows, GetClassNameW, GetWindowTextW, IsWindowVisible, SW_HIDE, SW_RESTORE,
            SetForegroundWindow, ShowWindow,
        },
    },
    core::{BOOL, PCWSTR},
};

static DEDICATED_CONSOLE: AtomicBool = AtomicBool::new(false);
static VISIBLE: AtomicBool = AtomicBool::new(false);
static WINDOW_TITLE: OnceLock<Vec<u16>> = OnceLock::new();

pub fn initialize() {
    if !owns_console() {
        return;
    }

    DEDICATED_CONSOLE.store(true, Ordering::Relaxed);
    let title = format!("GitButler Debug Console ({})", std::process::id())
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    unsafe {
        let _ = SetConsoleTitleW(PCWSTR(title.as_ptr()));
    }
    WINDOW_TITLE
        .set(title)
        .expect("debug console initialized only once");
}

pub fn hide_at_startup() {
    if !DEDICATED_CONSOLE.load(Ordering::Relaxed) {
        tracing::warn!("debug console is shared with another process; leaving it visible");
        return;
    }

    for attempt in 1..=100 {
        if let Some(window) = terminal_window() {
            let was_visible = unsafe { ShowWindow(window, SW_HIDE).as_bool() };
            let visible_after = unsafe { IsWindowVisible(window).as_bool() };
            VISIBLE.store(false, Ordering::Relaxed);
            tracing::info!(
                window = window.0 as usize,
                attempt,
                was_visible,
                visible_after,
                "hid dedicated Windows Terminal debug console"
            );
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    tracing::warn!("could not find the dedicated Windows Terminal window by class and title");
}

pub fn toggle() {
    if !DEDICATED_CONSOLE.load(Ordering::Relaxed) {
        tracing::warn!("refusing to hide a console shared with another process");
        return;
    }

    if !set_visible(!VISIBLE.load(Ordering::Relaxed)) {
        tracing::warn!("dedicated Windows Terminal window is unavailable");
    }
}

fn set_visible(visible: bool) -> bool {
    let Some(window) = terminal_window() else {
        return false;
    };

    unsafe {
        let _ = ShowWindow(window, if visible { SW_RESTORE } else { SW_HIDE });
        if visible {
            let _ = SetForegroundWindow(window);
        }
    }
    VISIBLE.store(visible, Ordering::Relaxed);
    true
}

fn terminal_window() -> Option<HWND> {
    struct Search {
        title: &'static [u16],
        result: Option<HWND>,
    }

    unsafe extern "system" fn callback(window: HWND, state: LPARAM) -> BOOL {
        let state = unsafe { &mut *(state.0 as *mut Search) };

        let mut class_name = [0; 64];
        let class_name_length = unsafe { GetClassNameW(window, &mut class_name) } as usize;
        if String::from_utf16_lossy(&class_name[..class_name_length])
            != "CASCADIA_HOSTING_WINDOW_CLASS"
        {
            return true.into();
        }

        let mut title = [0; 256];
        let length = unsafe { GetWindowTextW(window, &mut title) } as usize;
        if title[..length] == state.title[..state.title.len() - 1] {
            state.result = Some(window);
            return false.into();
        }
        true.into()
    }

    let title = WINDOW_TITLE.get()?;
    let mut state = Search {
        title,
        result: None,
    };
    unsafe {
        let _ = EnumWindows(
            Some(callback),
            LPARAM(std::ptr::from_mut(&mut state) as isize),
        );
    }
    state.result
}

fn owns_console() -> bool {
    let mut process_ids = [0; 2];
    unsafe { GetConsoleProcessList(&mut process_ids) == 1 }
}
