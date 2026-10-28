use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppItem {
    pub id: String,          // e.g., "7zip.7zip" or "Mozilla.Firefox"
    pub name: String,        // e.g., "7-Zip"
    pub category: String,    // e.g., "Utilities", "Browsers", "Runtimes"
    pub silent_args: String, // e.g., "/S" or "/qn /norestart"
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CttWinUtilExport {
    #[serde(default)]
    pub Install: Vec<String>,
    #[serde(default)]
    pub WPFTweaks: Vec<String>,
    #[serde(default)]
    pub WPFFeature: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildReceipt {
    pub iso_input_path: PathBuf,
    pub iso_output_path: PathBuf,
    pub selected_wim_index: u32,
    pub username: String,
    pub password: String,
    pub hostname: String,
    pub ultimate_performance: bool,
    pub strip_defender: bool,
    pub strip_edge: bool,
    pub strip_onedrive: bool,
    pub strip_telemetry: bool,
    pub ctt_json_path: Option<PathBuf>,
    pub winhance_config_path: Option<PathBuf>,
}

impl BuildReceipt {
    pub fn load_from_toml(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }

    pub fn save_to_toml(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

/// Parses an exported Chris Titus Tech (CTT) WinUtil JSON file
pub fn parse_ctt_export(path: &Path) -> Result<CttWinUtilExport> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("Failed to read CTT JSON export at {}", path.display()))?;
    let parsed: CttWinUtilExport = serde_json::from_str(&raw)
        .context("Invalid CTT WinUtil JSON format")?;
    Ok(parsed)
}

/// Default built-in curated catalog (matches CTT/Winhance essentials)
pub fn default_app_catalog() -> Vec<AppItem> {
    vec![
        AppItem { id: "7zip.7zip".into(), name: "7-Zip".into(), category: "Utilities".into(), silent_args: "/S".into(), selected: true },
        AppItem { id: "Mozilla.Firefox".into(), name: "Firefox".into(), category: "Browsers".into(), silent_args: "/S".into(), selected: false },
        AppItem { id: "Brave.Brave".into(), name: "Brave Browser".into(), category: "Browsers".into(), silent_args: "--silent --system-level".into(), selected: false },
        AppItem { id: "Microsoft.VCRedist.2015+.x64".into(), name: "VC++ Redist 2015-2022 (x64)".into(), category: "Runtimes".into(), silent_args: "/install /quiet /norestart".into(), selected: true },
        AppItem { id: "Microsoft.DirectX".into(), name: "DirectX End-User Runtime".into(), category: "Runtimes".into(), silent_args: "/Q".into(), selected: false },
        AppItem { id: "VideoLAN.VLC".into(), name: "VLC Media Player".into(), category: "Media".into(), silent_args: "/S".into(), selected: false },
        AppItem { id: "Notepad++.Notepad++".into(), name: "Notepad++".into(), category: "Development".into(), silent_args: "/S".into(), selected: true },
        AppItem { id: "Git.Git".into(), name: "Git for Windows".into(), category: "Development".into(), silent_args: "/VERYSILENT /NORESTART".into(), selected: false },
        AppItem { id: "Microsoft.PowerToys".into(), name: "Microsoft PowerToys".into(), category: "Utilities".into(), silent_args: "--silent".into(), selected: false },
        AppItem { id: "voidtools.Everything".into(), name: "Everything Search".into(), category: "Utilities".into(), silent_args: "-install-options \"-app-data -install-service\"".into(), selected: true },
    ]
}
