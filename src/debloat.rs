use anyhow::Result;
use std::path::Path;
use std::process::Command;

/// Appx packages targeted for removal across Win 10 & Win 11 Consumer ISOs.
/// On LTSC, DISM simply skips any package not present without throwing an error.
pub const BLOAT_APPX_PATTERNS: &[&str] = &[
    "microsoft.bingnews",
    "microsoft.bingweather",
    "microsoft.gamingapp",
    "microsoft.gethelp",
    "microsoft.getstarted",
    "microsoft.microsoftofficehub",
    "microsoft.microsoftsolitairecollection",
    "microsoft.MixedReality.Portal",
    "microsoft.people",
    "microsoft.powerautomate",
    "microsoft.skypeapp",
    "microsoft.todos",
    "microsoft.windowsalarms",
    "microsoft.windowscommunicationsapps",
    "microsoft.windowsfeedbackhub",
    "microsoft.windowsmaps",
    "microsoft.yourphone",
    "microsoft.zunevideo",
    "microsoft.zunemusic",
    "clipchamp.clipchamp",
    "microsoft.copilot",
    "microsoft.windows.ai.copilot.provider",
    "microsoft.devhome",
    "microsoft.outlookforwindows",
    "microsoftteams",
];

pub fn strip_provisioned_bloat(mount_dir: &Path, extra_patterns: &[String]) -> Result<()> {
    let mount_str = mount_dir.to_string_lossy();
    let output = Command::new("dism.exe")
        .args(["/English", &format!("/Image:{}", mount_str), "/Get-ProvisionedAppxPackages"])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Some(pkg) = line.trim().strip_prefix("PackageName : ") {
            let pkg_lower = pkg.trim().to_lowercase();
            let should_remove = BLOAT_APPX_PATTERNS.iter().any(|p| pkg_lower.contains(p))
                || extra_patterns.iter().any(|p| pkg_lower.contains(&p.to_lowercase()));

            if should_remove {
                let _ = Command::new("dism.exe")
                    .args([
                        "/English",
                        &format!("/Image:{}", mount_str),
                        "/Remove-ProvisionedAppxPackage",
                        &format!("/PackageName:{}", pkg.trim()),
                    ])
                    .status();
            }
        }
    }

    // Also strip Windows 11 24H2+ Recall AI capability if present
    let _ = Command::new("dism.exe")
        .args([
            "/English",
            &format!("/Image:{}", mount_str),
            "/Disable-Feature",
            "/FeatureName:Recall",
            "/Remove",
        ])
        .status();

    Ok(())
}

/// Mounts the offline WIM Registry Hives, applies CTT/Winhance & Telemetry tweaks, and unmounts cleanly
pub fn apply_offline_registry_tweaks(
    mount_dir: &Path,
    disable_telemetry: bool,
    remove_onedrive: bool,
    custom_reg_files: &[std::path::PathBuf],
) -> Result<()> {
    let software_hive = mount_dir.join("Windows\\System32\\config\\SOFTWARE");
    let system_hive = mount_dir.join("Windows\\System32\\config\\SYSTEM");
    let default_user_hive = mount_dir.join("Users\\Default\\NTUSER.DAT");

    // 1. Load Offline Hives
    run_reg(&["load", "HKLM\\WF_SOFTWARE", &software_hive.to_string_lossy()])?;
    run_reg(&["load", "HKLM\\WF_SYSTEM", &system_hive.to_string_lossy()])?;
    run_reg(&["load", "HKLM\\WF_NTUSER", &default_user_hive.to_string_lossy()])?;

    // 2. Apply Core CTT / Winhance Privacy & Performance Tweaks directly to the offline image
    if disable_telemetry {
        // Disable Data Collection / Telemetry across Win 10, Win 11, and LTSC
        let _ = run_reg(&["add", "HKLM\\WF_SOFTWARE\\Policies\\Microsoft\\Windows\\DataCollection", "/v", "AllowTelemetry", "/t", "REG_DWORD", "/d", "0", "/f"]);
        let _ = run_reg(&["add", "HKLM\\WF_SOFTWARE\\Policies\\Microsoft\\Windows\\CloudContent", "/v", "DisableWindowsConsumerFeatures", "/t", "REG_DWORD", "/d", "1", "/f"]);
        let _ = run_reg(&["add", "HKLM\\WF_SOFTWARE\\Policies\\Microsoft\\Windows\\WindowsAI", "/v", "DisableAIDataAnalysis", "/t", "REG_DWORD", "/d", "1", "/f"]);
        // Disable DiagTrack service in offline SYSTEM hive
        let _ = run_reg(&["add", "HKLM\\WF_SYSTEM\\ControlSet001\\Services\\DiagTrack", "/v", "Start", "/t", "REG_DWORD", "/d", "4", "/f"]);
        let _ = run_reg(&["add", "HKLM\\WF_SYSTEM\\ControlSet001\\Services\\dmwappushservice", "/v", "Start", "/t", "REG_DWORD", "/d", "4", "/f"]);
    }

    if remove_onedrive {
        let _ = run_reg(&["delete", "HKLM\\WF_NTUSER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run", "/v", "OneDriveSetup", "/f"]);
        let _ = run_reg(&["add", "HKLM\\WF_SOFTWARE\\Policies\\Microsoft\\Windows\\OneDrive", "/v", "DisableFileSyncNGSC", "/t", "REG_DWORD", "/d", "1", "/f"]);
    }

    // Classic Right-Click Context Menu on Windows 11 + Show File Extensions
    let _ = run_reg(&["add", "HKLM\\WF_NTUSER\\Software\\Classes\\CLSID\\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\\InprocServer32", "/ve", "/t", "REG_SZ", "/d", "", "/f"]);
    let _ = run_reg(&["add", "HKLM\\WF_NTUSER\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Advanced", "/v", "HideFileExt", "/t", "REG_DWORD", "/d", "0", "/f"]);

    // 3. Unload Hives (Garbage collect first via reg unload)
    let _ = run_reg(&["unload", "HKLM\\WF_NTUSER"]);
    let _ = run_reg(&["unload", "HKLM\\WF_SYSTEM"]);
    let _ = run_reg(&["unload", "HKLM\\WF_SOFTWARE"]);

    Ok(())
}

fn run_reg(args: &[&str]) -> Result<()> {
    Command::new("reg.exe").args(args).status()?;
    Ok(())
}
