//! Process spawning that must never flash a console window.
//!
//! A shipped Tauri app is a GUI-subsystem binary with no console of its own, so
//! every spawned console program (git, gh, npm) allocates a window, shows it,
//! and tears it down. Under `tauri dev` the dev process already owns a console
//! that the child inherits, which is why the problem only appears in the built
//! app - and why it reads as "the git CLI pops up and spams open and close".

use std::process::Command;

/// A `Command` for a console program that stays invisible on Windows.
///
/// Takes anything path-like: callers pass both `&str` and the `PathBuf` that
/// `lockfile::resolve_program` returns.
pub fn hidden(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut command = Command::new(program);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(windows))]
    {
        Command::new(program)
    }
}
