use crate::config::{AppItem, BuildReceipt, parse_ctt_export};
use crate::debloat::{apply_offline_registry_tweaks, strip_provisioned_bloat};
use crate::drivers::DriverEntry;
use crate::unattend::{UnattendProfile, generate_autounattend_xml};
use crate::wim_detect::WimImageInfo;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::sync::mpsc::UnboundedSender;

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

    if work_dir.exists() {
        let _ = fs::remove_dir_all(&work_dir);
    }
    fs::create_dir_all(&iso_extract_dir)?;
    fs::create_dir_all(&wim_mount_dir)?;

    // STEP 1: Extract Base ISO via 7z or tar
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

    // STEP 2: Convert install.esd to install.wim if needed, and export only the selected index to save space
    let _ = progress_tx.send((0.22, format!("Step 2/7: Preparing WIM image for {} (Index {})...", wim_info.name, wim_info.index)));
    let sources_dir = iso_extract_dir.join("sources");
    let esd_path = sources_dir.join("install.esd");
    let wim_path = sources_dir.join("install.wim");

    let mut active_wim_index = wim_info.index;
    if esd_path.exists() {
        Command::new("dism.exe")
            .args([
                "/English",
                "/Export-Image",
                &format!("/SourceImageFile:{}", esd_path.display()),
                &format!("/SourceIndex:{}", wim_info.index),
                &format!("/DestinationImageFile:{}", wim_path.display()),
                "/Compress:max",
                "/CheckIntegrity",
            ])
            .status()?;
        fs::remove_file(&esd_path)?;
        active_wim_index = 1; // Exported single edition becomes Index 1
    }

    // STEP 3: Mount install.wim
    let _ = progress_tx.send((0.35, "Step 3/7: Mounting install.wim for offline servicing...".into()));
    Command::new("dism.exe")
        .args([
            "/English",
            "/Mount-Wim",
            &format!("/WimFile:{}", wim_path.display()),
            &format!("/Index:{}", active_wim_index),
            &format!("/MountDir:{}", wim_mount_dir.display()),
        ])
        .status()?;

    // STEP 4: Inject Selected & Custom Override Drivers into install.wim
    let selected_drivers: Vec<&DriverEntry> = drivers.iter().filter(|d| d.selected).collect();
    let _ = progress_tx.send((0.50, format!("Step 4/7: Injecting {} selected drivers into offline image...", selected_drivers.len())));
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
    let _ = progress_tx.send((0.65, "Step 5/7: Stripping bloatware & injecting offline registry tweaks...".into()));
    strip_provisioned_bloat(&wim_mount_dir, &[])?;
    apply_offline_registry_tweaks(
        &wim_mount_dir,
        receipt.strip_telemetry,
        receipt.strip_onedrive,
        &[],
    )?;

    // Unmount and Commit WIM changes
    let _ = progress_tx.send((0.75, "Committing changes and unmounting install.wim...".into()));
    Command::new("dism.exe")
        .args([
            "/English",
            "/Unmount-Wim",
            &format!("/MountDir:{}", wim_mount_dir.display()),
            "/Commit",
        ])
        .status()?;

    // STEP 6: Stage $OEM$ Folder, Download Offline Apps via Winget, and Bundle CTT/Winhance Configs
    let _ = progress_tx.send((0.85, "Step 6/7: Packaging offline apps, CTT/Winhance configs & autounattend.xml...".into()));
    stage_oem_and_apps(&iso_extract_dir, &receipt, &wim_info, active_wim_index, &apps)?;

    // STEP 7: Master Bootable UEFI + Legacy BIOS ISO via oscdimg.exe
    let _ = progress_tx.send((0.95, "Step 7/7: Building bootable hybrid UEFI/BIOS ISO...".into()));
    build_bootable_iso(&iso_extract_dir, &receipt.iso_output_path)?;

    let _ = progress_tx.send((1.0, format!("Success! Custom ISO saved to {}", receipt.iso_output_path.display())));
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

    // 1. Download selected apps offline using `winget download`
    let mut selected_ids: Vec<String> = apps
        .iter()
        .filter(|a| a.selected)
        .map(|a| a.id.clone())
        .collect();

    // Merge apps from exported CTT WinUtil JSON if provided
    if let Some(ctt_path) = &receipt.ctt_json_path {
        if let Ok(ctt) = parse_ctt_export(ctt_path) {
            for id in ctt.Install {
                if !selected_ids.contains(&id) {
                    selected_ids.push(id);
                }
            }
        }
        fs::copy(ctt_path, configs_dir.join("ctt_export.json"))?;
    }

    for pkg_id in &selected_ids {
        let _ = Command::new("winget.exe")
            .args([
                "download",
                "--id",
                pkg_id,
                "--download-directory",
                &apps_dir.to_string_lossy(),
                "--accept-source-agreements",
                "--accept-package-agreements",
            ])
            .status();
    }

    // 2. Write `ei.cfg` into `sources\ei.cfg` so Windows Setup never prompts for a Product Key on any edition/LTSC
    let ei_cfg = if wim_info.is_ltsc {
        "[Channel]\r\nVolume\r\n[VL]\r\n1\r\n"
    } else {
        "[Channel]\r\n_Default\r\n[VL]\r\n0\r\n"
    };
    fs::write(iso_root.join("sources\\ei.cfg"), ei_cfg)?;

    // 3. Write `autounattend.xml` to ISO root
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

    // 4. Copy bootstrap.ps1 into $OEM$
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
        .context("Failed to execute oscdimg.exe. Place oscdimg.exe in the project root or System32.")?;

    if !status.success() {
        bail!("oscdimg.exe returned a non-zero exit code.");
    }
    Ok(())
}
