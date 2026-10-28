use crate::config::{AppItem, BuildReceipt, parse_ctt_export};
use crate::debloat::{apply_offline_registry_tweaks, strip_provisioned_bloat};
use crate::drivers::DriverEntry;
use crate::unattend::{UnattendProfile, generate_autounattend_xml};
use crate::wim_detect::WimImageInfo;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Serialize)]
struct OfflineAppManifestEntry {
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "InstallerPath")]
    installer_path: String,
    #[serde(rename = "SilentArgs")]
    silent_args: String,
}

/// RAII Guard: Guarantees DISM mounts and Registry Hives are cleaned up even if the build errors out
struct WimMountGuard {
    mount_dir: PathBuf,
    committed: bool,
}

impl Drop for WimMountGuard {
    fn drop(&mut self) {
        if !self.committed && self.mount_dir.exists() {
            let _ = Command::new("reg.exe").args(["unload", "HKLM\\WF_NTUSER"]).status();
            let _ = Command::new("reg.exe").args(["unload", "HKLM\\WF_SYSTEM"]).status();
            let _ = Command::new("reg.exe").args(["unload", "HKLM\\WF_SOFTWARE"]).status();
            let _ = Command::new("dism.exe")
                .args([
                    "/English",
                    "/Unmount-Wim",
                    &format!("/MountDir:{}", self.mount_dir.display()),
                    "/Discard",
                ])
                .status();
            let _ = Command::new("dism.exe").args(["/Cleanup-Wim"]).status();
        }
    }
}

pub fn execute_build_pipeline(
    receipt: BuildReceipt,
    wim_info: WimImageInfo,
    drivers: Vec<DriverEntry>,
    apps: Vec<AppItem>,
    progress_tx: UnboundedSender<(f32, String)>,
) -> Result<()> {
    let work_dir = std::env::temp_dir().join("WinForge_Workspace");
    let iso_extract_dir = work_dir.join("ISO_Root");
    let wim_mount_dir = work_dir.join("Mount_Install");
    let boot_mount_dir = work_dir.join("Mount_Boot");

    // Clean up any stale mounts from previous interrupted runs
    let _ = Command::new("dism.exe").args(["/Cleanup-Wim"]).status();
    if work_dir.exists() {
        let _ = fs::remove_dir_all(&work_dir);
    }
    fs::create_dir_all(&iso_extract_dir)?;
    fs::create_dir_all(&wim_mount_dir)?;
    fs::create_dir_all(&boot_mount_dir)?;

    // STEP 1: Extract Base ISO
    let _ = progress_tx.send((0.10, "Step 1/7: Extracting base Windows ISO...".into()));
    let status = Command::new("7z.exe")
        .args([
            "x",
            "-y",
            &receipt.iso_input_path.to_string_lossy(),
            &format!("-o{}", iso_extract_dir.display()),
        ])
        .status()
        .context("7z.exe not found in PATH. Install 7-Zip (`winget install 7zip.7zip`).")?;
    if !status.success() {
        bail!("Failed to extract ISO image.");
    }

    // STEP 2: Convert install.esd -> install.wim (or export single target index to shrink ISO size)
    let _ = progress_tx.send((
        0.22,
        format!("Step 2/7: Exporting target edition {} (Index {})...", wim_info.name, wim_info.index),
    ));
    let sources_dir = iso_extract_dir.join("sources");
    let esd_path = sources_dir.join("install.esd");
    let wim_path = sources_dir.join("install.wim");
    let exported_wim = sources_dir.join("install_single.wim");

    let source_image = if esd_path.exists() { &esd_path } else { &wim_path };
    Command::new("dism.exe")
        .args([
            "/English",
            "/Export-Image",
            &format!("/SourceImageFile:{}", source_image.display()),
            &format!("/SourceIndex:{}", wim_info.index),
            &format!("/DestinationImageFile:{}", exported_wim.display()),
            "/Compress:max",
            "/CheckIntegrity",
        ])
        .status()?;

    if esd_path.exists() {
        fs::remove_file(&esd_path)?;
    }
    if wim_path.exists() {
        fs::remove_file(&wim_path)?;
    }
    fs::rename(&exported_wim, &wim_path)?;
    let active_wim_index = 1; // Exported WIM now has our target edition at Index 1

    // STEP 2.5: Inject Critical Boot/Storage Drivers into boot.wim (Index 2 - Windows Setup PE)
    let boot_wim_path = sources_dir.join("boot.wim");
    let boot_critical_drivers: Vec<&DriverEntry> = drivers
        .iter()
        .filter(|d| {
            d.selected
                && matches!(
                    d.class_name.to_lowercase().as_str(),
                    "hdc" | "scsiadapter" | "storage" | "system" | "net"
                )
        })
        .collect();

    if !boot_critical_drivers.is_empty() && boot_wim_path.exists() {
        let _ = progress_tx.send((
            0.30,
            format!("Injecting {} storage/network drivers into boot.wim (WinPE Setup)...", boot_critical_drivers.len()),
        ));
        let mut boot_guard = WimMountGuard {
            mount_dir: boot_mount_dir.clone(),
            committed: false,
        };
        Command::new("dism.exe")
            .args([
                "/English",
                "/Mount-Wim",
                &format!("/WimFile:{}", boot_wim_path.display()),
                "/Index:2",
                &format!("/MountDir:{}", boot_mount_dir.display()),
            ])
            .status()?;

        for drv in boot_critical_drivers {
            let _ = Command::new("dism.exe")
                .args([
                    "/English",
                    &format!("/Image:{}", boot_mount_dir.display()),
                    "/Add-Driver",
                    &format!("/Driver:{}", drv.inf_path.display()),
                    "/ForceUnsigned",
                ])
                .status();
        }

        Command::new("dism.exe")
            .args([
                "/English",
                "/Unmount-Wim",
                &format!("/MountDir:{}", boot_mount_dir.display()),
                "/Commit",
            ])
            .status()?;
        boot_guard.committed = true;
    }

    // STEP 3: Mount install.wim with transactional guard
    let _ = progress_tx.send((0.38, "Step 3/7: Mounting install.wim for offline servicing...".into()));
    let mut install_guard = WimMountGuard {
        mount_dir: wim_mount_dir.clone(),
        committed: false,
    };
    let mount_status = Command::new("dism.exe")
        .args([
            "/English",
            "/Mount-Wim",
            &format!("/WimFile:{}", wim_path.display()),
            &format!("/Index:{}", active_wim_index),
            &format!("/MountDir:{}", wim_mount_dir.display()),
        ])
        .status()?;
    if !mount_status.success() {
        bail!("Failed to mount install.wim.");
    }

    // STEP 4: Inject All Selected Drivers (Host + Custom Overrides) into install.wim
    let selected_drivers: Vec<&DriverEntry> = drivers.iter().filter(|d| d.selected).collect();
    let _ = progress_tx.send((
        0.52,
        format!("Step 4/7: Injecting {} drivers into install.wim...", selected_drivers.len()),
    ));
    for drv in selected_drivers {
        let _ = Command::new("dism.exe")
            .args([
                "/English",
                &format!("/Image:{}", wim_mount_dir.display()),
                "/Add-Driver",
                &format!("/Driver:{}", drv.inf_path.display()),
                "/ForceUnsigned",
            ])
            .status();
    }

    // STEP 5: Strip Provisioned Appx Bloatware & Inject Offline Registry Hives
    let _ = progress_tx.send((0.68, "Step 5/7: Stripping bloatware & injecting offline registry tweaks...".into()));
    strip_provisioned_bloat(&wim_mount_dir, &[])?;
    let mut custom_regs = Vec::new();
    if let Some(reg_path) = &receipt.winhance_config_path {
        if reg_path.extension().and_then(|s| s.to_str()) == Some("reg") {
            custom_regs.push(reg_path.clone());
        }
    }
    apply_offline_registry_tweaks(
        &wim_mount_dir,
        receipt.strip_telemetry,
        receipt.strip_onedrive,
        &custom_regs,
    )?;

    // Commit and unmount install.wim
    let _ = progress_tx.send((0.78, "Committing changes and unmounting install.wim...".into()));
    let unmount_status = Command::new("dism.exe")
        .args([
            "/English",
            "/Unmount-Wim",
            &format!("/MountDir:{}", wim_mount_dir.display()),
            "/Commit",
        ])
        .status()?;
    if !unmount_status.success() {
        bail!("Failed to commit and unmount install.wim.");
    }
    install_guard.committed = true;

    // STEP 6: Stage $OEM$, Download Offline Apps + manifest.json, and Bundle CTT/Winhance Configs
    let _ = progress_tx.send((0.88, "Step 6/7: Downloading offline apps, bundling CTT/Winhance & autounattend.xml...".into()));
    stage_oem_and_apps(&iso_extract_dir, &receipt, &wim_info, active_wim_index, &apps)?;

    // STEP 7: Master Bootable Hybrid UEFI/BIOS ISO
    let _ = progress_tx.send((0.96, "Step 7/7: Repacking bootable UEFI/BIOS ISO via oscdimg...".into()));
    build_bootable_iso(&iso_extract_dir, &receipt.iso_output_path)?;

    let _ = progress_tx.send((1.0, format!("Build Complete! Saved to {}", receipt.iso_output_path.display())));
    Ok(())
}

fn stage_oem_and_apps(
    iso_root: &Path,
    receipt: &BuildReceipt,
    wim_info: &WimImageInfo,
    active_wim_index: u32,
    apps: &[AppItem],
) -> Result<()> {
    let oem_base = iso_root.join("sources\\$OEM$\\$1\\Temp\\WinForge");
    let apps_dir = oem_base.join("apps");
    let configs_dir = oem_base.join("configs");
    fs::create_dir_all(&apps_dir)?;
    fs::create_dir_all(&configs_dir)?;

    let mut apps_to_fetch: Vec<AppItem> = apps.iter().filter(|a| a.selected).cloned().collect();

    // Merge apps from exported CTT WinUtil JSON if provided
    if let Some(ctt_path) = &receipt.ctt_json_path {
        if ctt_path.exists() {
            if let Ok(ctt) = parse_ctt_export(ctt_path) {
                for ctt_id in ctt.Install {
                    if !apps_to_fetch.iter().any(|a| a.id.eq_ignore_ascii_case(&ctt_id)) {
                        apps_to_fetch.push(AppItem {
                            id: ctt_id.clone(),
                            name: ctt_id,
                            category: "CTT Import".into(),
                            silent_args: "/S".into(),
                            selected: true,
                        });
                    }
                }
            }
            fs::copy(ctt_path, configs_dir.join("ctt_export.json"))?;

            // Download raw winutil.ps1 so CTT config executes offline during FirstLogon
            let _ = Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-Command",
                    &format!(
                        "Invoke-WebRequest -Uri 'https://christitus.com/win' -OutFile '{}'",
                        configs_dir.join("winutil.ps1").display()
                    ),
                ])
                .status();
        }
    }

    // Copy Winhance .reg or .ps1 export if provided
    if let Some(wh_path) = &receipt.winhance_config_path {
        if wh_path.exists() {
            let ext = wh_path.extension().and_then(|s| s.to_str()).unwrap_or("reg");
            let dest_name = if ext == "ps1" { "winhance_export.ps1" } else { "winhance_tweaks.reg" };
            fs::copy(wh_path, configs_dir.join(dest_name))?;
        }
    }

    // Download each app into its own subfolder and build manifest.json for bootstrap.ps1
    let mut manifest_entries = Vec::new();
    for app in &apps_to_fetch {
        let pkg_sub_dir = apps_dir.join(&app.id);
        fs::create_dir_all(&pkg_sub_dir)?;

        let status = Command::new("winget.exe")
            .args([
                "download",
                "--id",
                &app.id,
                "--download-directory",
                &pkg_sub_dir.to_string_lossy(),
                "--accept-source-agreements",
                "--accept-package-agreements",
            ])
            .status();

        if status.map(|s| s.success()).unwrap_or(false) {
            if let Ok(entries) = fs::read_dir(&pkg_sub_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                    if matches!(ext.as_str(), "exe" | "msi" | "msixbundle" | "appxbundle") {
                        let file_name = path.file_name().unwrap().to_string_lossy();
                        let target_c_path = format!("C:\\Temp\\WinForge\\apps\\{}\\{}", app.id, file_name);
                        let silent_args = if ext == "msi" {
                            format!("/i \"{}\" /qn /norestart", target_c_path)
                        } else {
                            app.silent_args.clone()
                        };
                        let installer_exec = if ext == "msi" {
                            "msiexec.exe".to_string()
                        } else {
                            target_c_path
                        };

                        manifest_entries.push(OfflineAppManifestEntry {
                            name: app.name.clone(),
                            installer_path: installer_exec,
                            silent_args,
                        });
                    }
                }
            }
        }
    }

    let manifest_json = serde_json::to_string_pretty(&manifest_entries)?;
    fs::write(apps_dir.join("manifest.json"), manifest_json)?;

    // Write `ei.cfg` into `sources\ei.cfg` to bypass Product Key prompts on all editions/LTSC
    let ei_cfg = if wim_info.is_ltsc {
        "[Channel]\r\nVolume\r\n[VL]\r\n1\r\n"
    } else {
        "[Channel]\r\n_Default\r\n[VL]\r\n0\r\n"
    };
    fs::write(iso_root.join("sources\\ei.cfg"), ei_cfg)?;

    // Write `autounattend.xml`
    let mut adjusted_wim = wim_info.clone();
    adjusted_wim.index = active_wim_index;
    let profile = UnattendProfile {
        username: receipt.username.clone(),
        password: receipt.password.clone(),
        hostname: receipt.hostname.clone(),
        timezone: "India Standard Time".into(),
        locale: "en-US".into(),
        ultimate_performance: receipt.ultimate_performance,
    };
    let xml = generate_autounattend_xml(&adjusted_wim, &profile);
    fs::write(iso_root.join("autounattend.xml"), xml)?;

    // Write `bootstrap.ps1`
    fs::write(
        oem_base.join("bootstrap.ps1"),
        include_str!("../assets/scripts/bootstrap.ps1"),
    )?;

    Ok(())
}

fn build_bootable_iso(iso_root: &Path, output_iso: &Path) -> Result<()> {
    let etfsboot = iso_root.join("boot\\etfsboot.com");
    let efisys = iso_root.join("efi\\microsoft\\boot\\efisys.bin");

    let bootdata_arg = format!(
        "-bootdata:2#p0,e,b\"{}\"#{},e,b\"{}\"",
        etfsboot.display(),
        "pEF",
        efisys.display()
    );

    let status = Command::new("oscdimg.exe")
        .args([
            "-m",
            "-o",
            "-u2",
            "-udfver102",
            &bootdata_arg,
            &iso_root.to_string_lossy(),
            &output_iso.to_string_lossy(),
        ])
        .status()
        .context("Failed to execute oscdimg.exe. Ensure Windows ADK Deployment Tools or oscdimg.exe is in PATH.")?;

    if !status.success() {
        bail!("oscdimg.exe returned a non-zero exit code.");
    }
    Ok(())
}
