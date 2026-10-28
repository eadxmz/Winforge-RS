use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WimImageInfo {
    pub index: u32,
    pub name: String,
    pub edition_id: String,
    pub build_number: u32,
    pub is_win11: bool,
    pub is_ltsc: bool,
}

pub fn get_wim_editions(wim_path: &Path) -> Result<Vec<WimImageInfo>> {
    let output = Command::new("dism.exe")
        .args([
            "/English",
            &format!("/Get-WimInfo"),
            &format!("/WimFile:{}", wim_path.display()),
        ])
        .output()
        .context("Failed to execute DISM to read WIM indices")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut editions = Vec::new();
    let mut current_index = 0;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(idx_str) = trimmed.strip_prefix("Index : ") {
            current_index = idx_str.parse::<u32>().unwrap_or(0);
        } else if let Some(name_str) = trimmed.strip_prefix("Name : ") {
            if current_index > 0 {
                let details = get_single_index_details(wim_path, current_index, name_str)?;
                editions.push(details);
            }
        }
    }
    Ok(editions)
}

fn get_single_index_details(wim_path: &Path, index: u32, name: &str) -> Result<WimImageInfo> {
    let output = Command::new("dism.exe")
        .args([
            "/English",
            "/Get-WimInfo",
            &format!("/WimFile:{}", wim_path.display()),
            &format!("/Index:{}", index),
        ])
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut edition_id = String::new();
    let mut build_number = 19045; // Default fallback Win10

    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(ed) = trimmed.strip_prefix("Edition : ") {
            edition_id = ed.trim().to_string();
        } else if let Some(ver) = trimmed.strip_prefix("Version : ") {
            // Format: 10.0.26100 or 10.0.19044
            let parts: Vec<&str> = ver.trim().split('.').collect();
            if parts.len() >= 3 {
                build_number = parts[2].parse::<u32>().unwrap_or(19045);
            }
        }
    }

    let is_win11 = build_number >= 22000;
    let is_ltsc = edition_id.contains("EnterpriseS")
        || name.to_uppercase().contains("LTSC")
        || edition_id.contains("IoTEnterpriseS");

    Ok(WimImageInfo {
        index,
        name: name.to_string(),
        edition_id,
        build_number,
        is_win11,
        is_ltsc,
    })
}
