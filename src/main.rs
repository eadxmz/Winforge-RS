mod builder;
mod config;
mod debloat;
mod drivers;
mod unattend;
mod wim_detect;

use config::{BuildReceipt, default_app_catalog};
use drivers::{DriverEntry, DriverOrigin, override_driver};
use slint::{Model, ModelRc, SharedString, VecModel};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

slint::include_modules!();

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let ui = MainWindow::new()?;

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

    // Callback: Harvest Host Drivers from C:\Windows\System32\DriverStore\FileRepository
    let ui_weak = ui.as_weak();
    let drv_store_clone = driver_store.clone();
    ui.on_scan_host_drivers(move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        let repo_path = Path::new("C:\\Windows\\System32\\DriverStore\\FileRepository");
        let mut catalog = drv_store_clone.lock().unwrap();
        catalog.clear();

        if let Ok(entries) = std::fs::read_dir(repo_path) {
            for entry in entries.flatten().take(60) {
                // Scan third-partyoem .inf packages
                if let Ok(files) = std::fs::read_dir(entry.path()) {
                    for f in files.flatten() {
                        if f.path().extension().and_then(|s| s.to_str()) == Some("inf") {
                            if let Ok(drv) = DriverEntry::parse_from_inf(&f.path(), DriverOrigin::HostInstalled) {
                                catalog.push(drv);
                            }
                        }
                    }
                }
            }
        }
        refresh_driver_ui(&ui, &catalog);
        ui.set_status_text(format!("Harvested {} driver packages from host DriverStore.", catalog.len()).into());
    });

    // Callback: Override/Replace a Driver with a Custom .inf (e.g. older/newer AMD driver)
    let ui_weak = ui.as_weak();
    let drv_store_clone = driver_store.clone();
    ui.on_override_driver_inf(move |inf_path_str: SharedString| {
        let Some(ui) = ui_weak.upgrade() else { return };
        let path = PathBuf::from(inf_path_str.as_str());
        let mut catalog = drv_store_clone.lock().unwrap();

        match override_driver(&mut catalog, &path) {
            Ok(_) => {
                refresh_driver_ui(&ui, &catalog);
                ui.set_status_text(format!("Added custom override driver {} and unchecked conflicting host versions.", path.display()).into());
            }
            Err(e) => {
                ui.set_status_text(format!("Failed to parse INF: {}", e).into());
            }
        }
    });

    // Callback: Toggle individual driver selection
    let drv_store_clone = driver_store.clone();
    ui.on_toggle_driver(move |idx, checked| {
        if let Ok(mut catalog) = drv_store_clone.lock() {
            if let Some(d) = catalog.get_mut(idx as usize) {
                d.selected = checked;
            }
        }
    });

    // Callback: Toggle individual app selection
    let app_store_clone = app_store.clone();
    ui.on_toggle_app(move |idx, checked| {
        if let Ok(mut apps) = app_store_clone.lock() {
            if let Some(a) = apps.get_mut(idx as usize) {
                a.selected = checked;
            }
        }
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
