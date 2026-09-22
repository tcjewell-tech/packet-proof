#[derive(Debug, PartialEq, Eq)]
pub enum ElevationState {
    Elevated,
    NotElevated,
}

pub fn decide_relaunch(state: &ElevationState) -> bool {
    matches!(state, ElevationState::NotElevated)
}

#[cfg(target_os = "windows")]
pub fn check_elevation() -> ElevationState {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return ElevationState::NotElevated;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        if ok && elevation.TokenIsElevated != 0 {
            ElevationState::Elevated
        } else {
            ElevationState::NotElevated
        }
    }
}

#[cfg(target_os = "macos")]
pub fn check_elevation() -> ElevationState {
    if unsafe { libc::geteuid() } == 0 {
        ElevationState::Elevated
    } else {
        ElevationState::NotElevated
    }
}

#[cfg(target_os = "windows")]
pub fn relaunch_elevated() -> ! {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let exe = std::env::current_exe().expect("current_exe");
    let exe_wide: Vec<u16> = OsStr::new(&exe).encode_wide().chain(std::iter::once(0)).collect();
    let verb: Vec<u16> = OsStr::new("runas").encode_wide().chain(std::iter::once(0)).collect();

    unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(exe_wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
    std::process::exit(0)
}

#[cfg(target_os = "macos")]
pub fn relaunch_elevated() -> ! {
    let exe = std::env::current_exe().expect("current_exe");
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        exe.display()
    );
    let _ = std::process::Command::new("osascript").arg("-e").arg(script).status();
    std::process::exit(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relaunches_when_not_elevated() {
        assert!(decide_relaunch(&ElevationState::NotElevated));
    }

    #[test]
    fn does_not_relaunch_when_already_elevated() {
        assert!(!decide_relaunch(&ElevationState::Elevated));
    }
}
