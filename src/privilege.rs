/// Check if the current process is running with administrator privileges.
/// On non-Windows platforms, always returns true (for development).
pub fn is_elevated() -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        let output = Command::new("powershell")
            .args(["-NoProfile", "-Command", "([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)"])
            .output();
        match output {
            Ok(o) => String::from_utf8_lossy(&o.stdout).trim() == "True",
            Err(_) => false,
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        true // For development on Linux/macOS
    }
}
