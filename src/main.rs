#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod builder;
mod config;
mod debloat;
mod drivers;
mod toolchain;
mod unattend;
mod wim_detect;

use builder::execute_build_pipeline;
use config::{BuildReceipt, default_app_catalog};
use drivers::{DriverEntry, DriverOrigin, override_driver};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use wim_detect::{WimImageInfo, get_wim_editions};

slint::include_modules!();

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let ui = MainWindow::new()?;

    let wim_editions: Arc<Mutex<Vec<WimImageInfo>>> = Arc::new(Mutex::new( vec![
        WimImageInfo {
            index: 1,
            name: "Windows 11 Enterprise LTSC".into(),
            edition_id: "EnterpriseS".into(),
            build_number: 26100,
            is_win11: true,
            is_ltsc: true,
        },
        WimImageInfo {
            index: 2,
            name: "Windows 11 IoT Enterprise LTSC".into(),
            edition_id: "IoTEnterpriseS".into(),
            build_number: 26100,
            is_win11: true,
            is_ltsc: true,
        },
    ]));
    let driver_store: Arc<Mutex<Vec<DriverEntry>>> = Arc::new(Mutex::new(Vec::new()));
    let app_store = Arc::new(Mutex::new(default_app_catalog()));

    // Populate initial App Catalog into UI
    {
        let apps = app_store.lock().unwrap();
        let ui_apps: Vec<UiAppItem> = apps
            .iter()
            .map(|a| UiAppItem {
                id: a.id.clone().into(),
                name: a.name.clone().into(),
                category: a.category.clone().into(),
                selected: a.selected,
            })
            .collect();
        ui.set_apps(ModelRc::from(Rc::new(VecModel::from(ui_apps))));
    }

    // 1. Callback: Inspect ISO / WIM Editions
    let ui_weak = ui.as_weak();
    let editions_clone = wim_editions.clone();
    ui.on_scan_iso(move |iso_path_str: SharedString| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let path = PathBuf::from(iso_path_str.as_str());

        if !path.exists() {
            ui.set_status_text("ISO or WIM file not found at specified path.".into());
            return;
        }

        // If user points directly to a mounted ISO drive or WIM/ESD, inspect immediately;
        // otherwise extract sources/install.wim header or mount ISO via PowerShell Mount-DiskImage
        ui.set_status_text("Mounting ISO to inspect WIM/ESD indices...".into());
        let mount_out = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "$m = Mount-DiskImage -ImagePath '{}' -PassThru; ($m | Get-Volume).DriveLetter",
                    path.display()
                ),
            ])
            .output();

        if let Ok(out) = mount_out {
            let drive_letter = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !drive_letter.is_empty() {
                let wim_candidate = PathBuf::from(format!("{}:\\sources\\install.wim", drive_letter));
                let esd_candidate = PathBuf::from(format!("{}:\\sources\\install.esd", drive_letter));
                let target = if wim_candidate.exists() { wim_candidate } else { esd_candidate };

                if let Ok(detected) = get_wim_editions(&target) {
                    let labels: Vec<SharedString> = detected
                        .iter()
                        .map(|e| {
                            format!(
                                "Index {}: {} [{} - Build {}]{}",
                                e.index,
                                e.name,
                                e.edition_id,
                                e.build_number,
                                if e.is_ltsc { " (LTSC)" } else { "" }
                            )
                            .into()
                        })
                        .collect();
                    ui.set_edition_list(ModelRc::from(Rc::new(VecModel::from(labels))));
                    ui.set_selected_edition_idx(0);
                    *editions_clone.lock().unwrap() = detected;
                    ui.set_status_text("Successfully parsed all WIM/ESD editions from ISO.".into());
                }

                let _ = std::process::Command::new("powershell.exe")
                    .args([
                        "-NoProfile",
                        "-Command",
                        &format!("Dismount-DiskImage -ImagePath '{}'", path.display()),
                    ])
                    .status();
            }
        }
    });

    // 2. Callback: Harvest Host Drivers
    let ui_weak = ui.as_weak();
    let drv_store_clone = driver_store.clone();
    ui.on_scan_host_drivers(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let repo_path = Path::new("C:\\Windows\\System32\\DriverStore\\FileRepository");
        let mut catalog = drv_store_clone.lock().unwrap();
        catalog.clear();

        if let Ok(entries) = std::fs::read_dir(repo_path) {
            for entry in entries.flatten() {
                let folder_name = entry.file_name().to_string_lossy().to_lowercase();
                // Skip built-in Microsoft inbox drivers; only harvest 3rd-party OEM drivers
                if folder_name.starts_with("oem") || !folder_name.starts_with("ms") {
                    if let Ok(files) = std::fs::read_dir(entry.path()) {
                        for f in files.flatten() {
                            if f.path().extension().and_then(|s| s.to_str()) == Some("inf") {
                                if let Ok(drv) = DriverEntry::parse_from_inf(&f.path(), DriverOrigin::HostInstalled) {
                                    if !drv.provider.to_lowercase().contains("microsoft") {
                                        catalog.push(drv);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        refresh_driver_ui(&ui, &catalog);
        ui.set_status_text(format!("Harvested {} third-party OEM driver packages from host DriverStore.", catalog.len()).into());
    });

    // 3. Callback: Override/Replace a Driver with Custom .inf
    let ui_weak = ui.as_weak();
    let drv_store_clone = driver_store.clone();
    ui.on_override_driver_inf(move |inf_path_str: SharedString| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let path = PathBuf::from(inf_path_str.as_str());
        let mut catalog = drv_store_clone.lock().unwrap();

        match override_driver(&mut catalog, &path) {
            Ok(_) => {
                refresh_driver_ui(&ui, &catalog);
                ui.set_status_text(format!("Added custom override {} and disabled conflicting host driver.", path.display()).into());
            }
            Err(e) => {
                ui.set_status_text(format!("Error parsing INF: {}", e).into());
            }
        }
    });

    // 4. Toggle callbacks
    let drv_store_clone = driver_store.clone();
    ui.on_toggle_driver(move |idx, checked| {
        if let Ok(mut catalog) = drv_store_clone.lock() {
            if let Some(d) = catalog.get_mut(idx as usize) {
                d.selected = checked;
            }
        }
    });

    let app_store_clone = app_store.clone();
    ui.on_toggle_app(move |idx, checked| {
        if let Ok(mut apps) = app_store_clone.lock() {
            if let Some(a) = apps.get_mut(idx as usize) {
                a.selected = checked;
            }
        }
    });

    // 5. Callback: Start Async Build Pipeline
    let ui_weak = ui.as_weak();
    let editions_clone = wim_editions.clone();
    let drv_store_clone = driver_store.clone();
    let app_store_clone = app_store.clone();

    ui.on_start_build(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        ui.set_is_building(true);
        ui.set_build_progress(0.02);

        let selected_idx = ui.get_selected_edition_idx() as usize;
        let editions = editions_clone.lock().unwrap();
        let target_wim = editions.get(selected_idx).cloned().unwrap_or_else(|| editions[0].clone());
        drop(editions);

        let ctt_str = ui.get_ctt_config_path().to_string();
        let wh_str = ui.get_winhance_config_path().to_string();

        let receipt = BuildReceipt {
            iso_input_path: PathBuf::from(ui.get_iso_path().as_str()),
            iso_output_path: PathBuf::from(ui.get_output_iso_path().as_str()),
            selected_wim_index: target_wim.index,
            username: ui.get_username().to_string(),
            password: ui.get_password().to_string(),
            hostname: ui.get_hostname().to_string(),
            ultimate_performance: ui.get_ultimate_perf(),
            strip_defender: false,
            strip_edge: ui.get_strip_edge(),
            strip_onedrive: ui.get_strip_onedrive(),
            strip_telemetry: ui.get_strip_telemetry(),
            ctt_json_path: if ctt_str.trim().is_empty() { None } else { Some(PathBuf::from(ctt_str)) },
            winhance_config_path: if wh_str.trim().is_empty() { None } else { Some(PathBuf::from(wh_str)) },
        };

        let drivers_snapshot = drv_store_clone.lock().unwrap().clone();
        let apps_snapshot = app_store_clone.lock().unwrap().clone();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(f32, String)>();
        let ui_weak_progress = ui.as_weak();

        // Spawn UI progress listener
        tokio::spawn(async move {
            while let Some((prog, msg)) = rx.recv().await {
                let ui_handle = ui_weak_progress.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_handle.upgrade() {
                        ui.set_build_progress(prog);
                        ui.set_status_text(msg.into());
                    }
                });
            }
        });

        // Spawn blocking DISM/ISO build thread
        let ui_weak_done = ui.as_weak();
        tokio::task::spawn_blocking(move || {
            let res = execute_build_pipeline(receipt, target_wim, drivers_snapshot, apps_snapshot, tx);
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_weak_done.upgrade() {
                    ui.set_is_building(false);
                    if let Err(e) = res {
                        ui.set_status_text(format!("Build Error: {:#}", e).into());
                    }
                }
            });
        });
    });

    ui.run()?;
    Ok(())
}

fn refresh_driver_ui(ui: &MainWindow, catalog: &[DriverEntry]) {
    let ui_list: Vec<UiDriverItem> = catalog
        .iter()
        .map(|d| UiDriverItem {
            inf_name: d.inf_name.clone().into(),
            provider: d.provider.clone().into(),
            class_name: d.class_name.clone().into(),
            version: d.version.clone().into(),
            date: d.date.clone().into(),
            origin: match d.origin {
                DriverOrigin::HostInstalled => "Host Installed".into(),
                DriverOrigin::UserCustomOverride => "Custom Override".into(),
            },
            selected: d.selected,
        })
        .collect();
    ui.set_drivers(ModelRc::from(Rc::new(VecModel::from(ui_list))));
}
