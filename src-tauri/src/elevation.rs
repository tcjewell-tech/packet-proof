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

/// Shown when elevation could not be obtained, on either platform.
///
/// Without it the app simply vanishes when the user declines the UAC or
/// Touch ID prompt, which reads as a crash rather than as the deliberate
/// security gate it is.
const ELEVATION_DECLINED_MESSAGE: &str = "Packet Proof needs administrator privileges to run.\n\n\
It sends and receives raw ICMP packets to trace the route between this \
computer and a target, and the operating system only allows raw sockets to \
elevated processes.\n\n\
Packet Proof cannot continue without them. Start it again and approve the \
prompt if you want to use it.";

#[cfg(target_os = "windows")]
fn show_elevation_declined_dialog() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK};

    let to_wide = |s: &str| -> Vec<u16> { OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect() };
    let text = to_wide(ELEVATION_DECLINED_MESSAGE);
    let caption = to_wide("Packet Proof");

    unsafe {
        MessageBoxW(None, PCWSTR(text.as_ptr()), PCWSTR(caption.as_ptr()), MB_OK | MB_ICONWARNING);
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

    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(exe_wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW returns a pseudo-HINSTANCE: anything <= 32 is an error
    // code, not a handle. Declining the UAC prompt gives
    // SE_ERR_ACCESSDENIED (5).
    if !shell_execute_succeeded(result.0 as isize) {
        show_elevation_declined_dialog();
    }
    std::process::exit(0)
}

/// `ShellExecuteW`'s documented success test: the returned value is "greater
/// than 32 if successful", otherwise it is an error code.
#[cfg(target_os = "windows")]
fn shell_execute_succeeded(result: isize) -> bool {
    result > 32
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
    let status = std::process::Command::new("osascript").arg("-e").arg(script).status();

    // Cancelling the `with administrator privileges` auth prompt makes
    // AppleScript raise error -128 ("User canceled"), so `osascript` exits
    // non-zero and prints "execution error: User canceled. (-128)" to
    // stderr. Run non-interactively like this it does *not* put up its own
    // dialog — the message only goes to a stderr nobody is reading — so
    // without this branch the app just disappears. Failing to spawn
    // osascript at all lands here too, which is equally worth explaining.
    let succeeded = matches!(status, Ok(s) if s.success());
    if !succeeded {
        show_elevation_declined_dialog();
    }
    std::process::exit(0)
}

#[cfg(target_os = "macos")]
fn show_elevation_declined_dialog() {
    // AppleScript string literal escaping, same two-parser care as above —
    // except the message is a compile-time constant, so only the AppleScript
    // layer is in play (no shell involved in `display dialog`).
    let escaped = ELEVATION_DECLINED_MESSAGE.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
    let script = format!("display dialog \"{escaped}\" buttons {{\"OK\"}} default button \"OK\" with icon caution with title \"Packet Proof\"");
    let _ = std::process::Command::new("osascript").arg("-e").arg(script).status();
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

    #[cfg(target_os = "windows")]
    #[test]
    fn shell_execute_results_at_or_below_32_are_failures() {
        // SE_ERR_ACCESSDENIED (5) is what declining the UAC prompt returns.
        assert!(!shell_execute_succeeded(5));
        assert!(!shell_execute_succeeded(0));
        assert!(!shell_execute_succeeded(32));
        assert!(shell_execute_succeeded(33));
        assert!(shell_execute_succeeded(42));
    }

    #[test]
    fn the_declined_message_explains_why_and_what_to_do() {
        assert!(ELEVATION_DECLINED_MESSAGE.contains("administrator privileges"));
        assert!(ELEVATION_DECLINED_MESSAGE.contains("ICMP"));
        assert!(ELEVATION_DECLINED_MESSAGE.contains("cannot continue"));
    }
}
