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
    // `do shell script` is TWO parsers deep: AppleScript parses the `-e`
    // script text (so the path must be escaped as an AppleScript string
    // literal), and then hands the resulting string to /bin/sh (so the
    // path also needs shell quoting, or a space/`;`/`$` etc. either breaks
    // a perfectly normal install path like "/Applications/Packet
    // Proof.app/..." or — worse, since this runs `with administrator
    // privileges` — lets a crafted path run arbitrary shell commands as
    // root). `quoted form of` is AppleScript's built-in for producing a
    // shell-safe single-quoted token at runtime, which handles the shell
    // layer; the string literal escaping below still handles the
    // AppleScript layer.
    let escaped_path = exe
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let script = format!(
        "do shell script quoted form of \"{escaped_path}\" with administrator privileges"
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
