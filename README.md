# WinForge-RS 🛠️💿

**WinForge-RS** is a declarative, zero-dependency Windows ISO Slipstreamer, Driver Version Swapper, Offline App Bundler, and Unattended Deployment Studio written in **Rust** and **Slint UI**.

It supports **all Windows 10 and Windows 11 editions**, including consumer (`Home`, `Pro`, `Workstation`) and enterprise long-term servicing channels (`Enterprise LTSC 2019/2021/2024` and `IoT Enterprise LTSC`).

---

## ✨ Core Features

* **Universal WIM & ESD Multi-Index Engine:** Automatically mounts and inspects multi-edition ISOs, converts compressed `install.esd` images to `install.wim`, and injects `sources\ei.cfg` + `<MetaData>` index targeting so Windows Setup never prompts for a product key or edition selection.
* **Smart Driver Harvester & Version Swap Studio:**
  * Scans `C:\Windows\System32\DriverStore\FileRepository` to extract third-party OEM drivers installed on the host.
  * Parses `.inf` files (`UTF-16LE` / `UTF-8` / `Windows-1252`) to extract `DriverVer`, `Provider`, and PCI/USB Hardware IDs (`PCI\VEN_...`).
  * **One-Click Driver Override:** Point to any older or newer `.inf` package (e.g., a specific AMD Radeon Adrenalin or Catalyst driver) and WinForge-RS automatically unchecks the conflicting host driver sharing the same Hardware ID.
  * **WinPE Boot Critical Injection:** Automatically injects selected `Storage`, `HDC`, `SCSIAdapter`, and `Net` drivers into `boot.wim` (Index 2) so NVMe/RAID/VMD drives are visible during partitioning.
* **100% Offline App Bundler (LTSC-Ready):**
  * Select apps from a built-in catalog or import a **Chris Titus Tech (CTT WinUtil)** JSON config.
  * Downloads offline `.exe`/`.msi` installers at build time into `sources\$OEM$\$1\Temp\WinForge\apps` with an auto-generated `manifest.json`—meaning apps install silently on first boot **even on Windows LTSC (where `winget` and the Microsoft Store are missing)** and without an internet connection.
* **Offline Registry Hive & Appx Debloater:**
  * Strips provisioned bloatware (`Copilot`, `Recall`, `Clipchamp`, `Teams`, `Xbox`, `Bing`) directly from the mounted `install.wim`.
  * Mounts offline `SOFTWARE`, `SYSTEM`, and Default User `NTUSER.DAT` hives prior to ISO creation to disable `DiagTrack`/Telemetry, block OneDrive setup, enable file extensions, and restore the classic Windows 11 context menu.
* **CTT WinUtil & Winhance Config Integration:**
  * Import exported **CTT WinUtil (`.json`)** and **Winhance (`.reg` / `.ps1`)** tweak presets directly into the ISO.
  * Unlocks and activates the hidden **Ultimate Performance** power plan (`e9a42b02-d5df-448d-aa00-03f14749eb61`) with zero standby/hibernate timeouts out-of-the-box.
* **Zero-Touch `autounattend.xml` Generator:**
  * Bypasses Windows 11 TPM 2.0, Secure Boot, RAM, and Microsoft Account (`OOBE\BypassNRO`) requirements automatically.
  * Pre-configures your local Administrator account, passwordless auto-logon, and hostname.
* **Self-Bootstrapping Toolchain:**
  * Automatically uses bundled `oscdimg.exe` and `7z.exe` from the portable release or downloads them on-demand into `%LOCALAPPDATA%\WinForge\bin` if run from source.

---

## 🚀 Quick Start (Portable Release)

1. Download the latest **`WinForge-RS-Portable-x64.zip`** from the [Releases](../../releases) page.
2. Extract the archive to any folder and run **`winforge-rs.exe`** (automatically prompts for Administrator elevation via embedded UAC manifest).
3. **Tab 1 (ISO & Edition):** Select your source Windows 10, 11, or LTSC `.iso` and click **Inspect WIM / ESD Editions**.
4. **Tab 2 (Driver Studio):** Click **Harvest Installed Host Drivers**, and optionally supply a custom `.inf` path to replace any specific driver (such as an older/newer AMD display driver).
5. **Tab 3 (Offline App Bundler):** Check the applications you want packaged directly into the ISO.
6. **Tab 4 (CTT / Winhance Tweaks):** Toggle Ultimate Performance, offline telemetry/bloatware removal, and attach your exported CTT `.json` or Winhance `.reg`/`.ps1` config.
7. **Tab 5 (Unattend & Build):** Enter your desired username and hostname, then click **Build Bootable Custom Windows ISO**.

---

## 🛠️ Building from Source

### Prerequisites
* **Rust Toolchain** (2024 Edition / Stable MSVC: `x86_64-pc-windows-msvc`)
* **Windows 10 or Windows 11** (Required for native `dism.exe` and `reg.exe` offline hive servicing)

```powershell
git clone [https://github.com/](https://github.com/)<your-username>/winforge-rs.git
cd winforge-rs
cargo build --release
