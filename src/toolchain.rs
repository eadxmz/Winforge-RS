use anyhow::{Context, Result, bail};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc::UnboundedSender;

pub struct ToolchainPaths {
    pub seven_zip: PathBuf,
    pub oscdimg: PathBuf,
}

const SEVEN_ZIP_STANDALONE_URL: &str = "https://www.7-zip.org/a/7zr.exe";

const OSCDIMG_URLS: &[&str] = &[
    "https://github.com/JpMicrosystems/oscdimg/raw/main/oscdimg.exe",
    "https://raw.githubusercontent.com/pbatard/rufus/master/res/oscdimg/oscdimg.exe",
];

pub fn ensure_toolchain(progress_tx: &UnboundedSender<(f32, String)>) -> Result<ToolchainPaths> {
    let bin_dir = get_local_bin_dir()?;
    fs::create_dir_all(&bin_dir)?;

    let seven_zip = resolve_or_download_7zip(&bin_dir, progress_tx)?;
    let oscdimg = resolve_or_download_oscdimg(&bin_dir, progress_tx)?;

    Ok(ToolchainPaths { seven_zip, oscdimg })
}

fn get_local_bin_dir() -> Result<PathBuf> {
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        Ok(PathBuf::from(local_app_data).join("WinForge").join("bin"))
    } else {
        Ok(std::env::temp_dir().join("WinForge_Bin"))
    }
}

fn resolve_or_download_7zip(
    bin_dir: &Path,
    progress_tx: &UnboundedSender<(f32, String)>,
) -> Result<PathBuf> {
    // 1. Check next to winforge-rs.exe (Portable ZIP bundle)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let bundled_7z = exe_dir.join("7z.exe");
            if bundled_7z.exists() {
                return Ok(bundled_7z);
            }
        }
    }

    // 2. Check %LOCALAPPDATA%\WinForge\bin
    let cached_7z = bin_dir.join("7z.exe");
    if cached_7z.exists() {
        return Ok(cached_7z);
    }
    let cached_7zr = bin_dir.join("7zr.exe");
    if cached_7zr.exists() {
        return Ok(cached_7zr);
    }

    // 3. Check standard Program Files installation
    let prog_7z = PathBuf::from("C:\\Program Files\\7-Zip\\7z.exe");
    if prog_7z.exists() {
        return Ok(prog_7z);
    }

    // 4. Fallback: Download standalone 7zr.exe automatically
    let _ = progress_tx.send((
        0.04,
        "Bootstrapping toolchain: Downloading standalone 7zr.exe...".into(),
    ));
    download_binary(SEVEN_ZIP_STANDALONE_URL, &cached_7zr)
        .context("Failed to auto-download standalone 7zr.exe")?;

    Ok(cached_7zr)
}

fn resolve_or_download_oscdimg(
    bin_dir: &Path,
    progress_tx: &UnboundedSender<(f32, String)>,
) -> Result<PathBuf> {
    // 1. Check next to winforge-rs.exe (Portable ZIP bundle)
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let local_oscdimg = exe_dir.join("oscdimg.exe");
            if local_oscdimg.exists() {
                return Ok(local_oscdimg);
            }
        }
    }

    // 2. Check %LOCALAPPDATA%\WinForge\bin\oscdimg.exe
    let cached_oscdimg = bin_dir.join("oscdimg.exe");
    if cached_oscdimg.exists() {
        return Ok(cached_oscdimg);
    }

    // 3. Check standard Windows ADK location if installed
    let adk_path = PathBuf::from(
        "C:\\Program Files (x86)\\Windows Kits\\10\\Assessment and Deployment Kit\\Deployment Tools\\amd64\\Oscdimg\\oscdimg.exe",
    );
    if adk_path.exists() {
        return Ok(adk_path);
    }

    // 4. Fallback: Download standalone oscdimg.exe automatically
    let _ = progress_tx.send((
        0.07,
        "Bootstrapping toolchain: Downloading Microsoft oscdimg.exe...".into(),
    ));

    let mut last_err = None;
    for url in OSCDIMG_URLS {
        match download_binary(url, &cached_oscdimg) {
            Ok(_) => return Ok(cached_oscdimg),
            Err(e) => last_err = Some(e),
        }
    }

    bail!(
        "Failed to auto-download oscdimg.exe from mirrors: {:?}",
        last_err
    )
}

fn download_binary(url: &str, dest: &Path) -> Result<()> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("WinForge-RS/0.1.0")
        .build()?;

    let response = client.get(url).send()?.error_for_status()?;
    let bytes = response.bytes()?;

    if bytes.len() < 64 || &bytes[0..2] != b"MZ" {
        bail!("Downloaded file from {} is not a valid Windows PE executable", url);
    }

    let mut file = fs::File::create(dest)?;
    file.write_all(&bytes)?;
    Ok(())
}
