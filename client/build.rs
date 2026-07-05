//! Windows: per-binary ICO from one master PNG per exe (Lanczos downscale from 256×256).
//!
//! Masters in `src/icons/`:
//!   backup-client.ico   → backup-client.exe
//!   backup-monitor.ico  → backup-monitor.exe

use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use image::GenericImageView;

const CLIENT_MASTER: &str = "src/icons/backup-client.ico";
const MONITOR_MASTER: &str = "src/icons/backup-monitor.ico";
const ICON_SIZES: [u32; 5] = [32, 48, 64, 128, 256];
const AUTHOR: &str = "Emelyanov Anton";
const COPYRIGHT: &str =
    "Copyright (C) 2026 Emelyanov Anton (geniden@gmail.com). https://github.com/geniden/backup";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={CLIENT_MASTER}");
    println!("cargo:rerun-if-changed={MONITOR_MASTER}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));

    let client_rc = prepare_resource(
        &manifest_dir,
        &out_dir.join("winres-client"),
        CLIENT_MASTER,
        "backup-client.exe",
        "Backup Client",
        "Backup Client",
    )
    .expect("backup-client icon");
    embed_resource::compile_for(client_rc, &["backup-client"], embed_resource::NONE);

    if std::env::var("CARGO_FEATURE_MONITOR").is_ok() {
        let monitor_rc = prepare_resource(
            &manifest_dir,
            &out_dir.join("winres-monitor"),
            MONITOR_MASTER,
            "backup-monitor.exe",
            "Backup Monitor",
            "Backup Monitor",
        )
        .expect("backup-monitor icon");
        embed_resource::compile_for(monitor_rc, &["backup-monitor"], embed_resource::NONE);
    }
}

fn prepare_resource(
    manifest_dir: &Path,
    work_dir: &Path,
    master_rel: &str,
    original_filename: &str,
    file_description: &str,
    product_name: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(work_dir)?;

    let app_icon = work_dir.join("app.ico");
    merge_icons(manifest_dir, master_rel, &app_icon)?;

    let icon_for_rc = app_icon
        .to_str()
        .ok_or("app.ico path is not valid UTF-8")?;

    let mut res = winres::WindowsResource::new();
    res.set_icon(icon_for_rc);
    apply_version_info(&mut res, original_filename, product_name, file_description);

    let rc_path = work_dir.join("resource.rc");
    res.write_resource_file(&rc_path)?;
    Ok(rc_path)
}

fn merge_icons(manifest_dir: &Path, master_rel: &str, out_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let master_path = manifest_dir.join(master_rel);
    let bytes = std::fs::read(&master_path).map_err(|e| {
        format!("read master icon {}: {e}", master_path.display())
    })?;
    let master = image::load_from_memory(&bytes).map_err(|e| {
        format!("decode master icon {}: {e}", master_path.display())
    })?;

    let (mw, mh) = master.dimensions();
    if mw != mh {
        eprintln!(
            "cargo:warning=master icon is {mw}x{mh}, expected square; using square crop"
        );
    }

    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);

    for size in ICON_SIZES {
        let resized = if mw == size && mh == size {
            master.to_rgba8()
        } else {
            master
                .resize_exact(size, size, FilterType::Lanczos3)
                .to_rgba8()
        };
        let icon_image = ico::IconImage::from_rgba_data(size, size, resized.into_raw());
        let entry = ico::IconDirEntry::encode(&icon_image).map_err(|e| {
            format!("encode ICO {size}x{size}: {e}")
        })?;
        dir.add_entry(entry);
    }

    let mut out = std::fs::File::create(out_path)?;
    dir.write(&mut out)?;
    Ok(())
}

fn apply_version_info(
    res: &mut winres::WindowsResource,
    original_filename: &str,
    product_name: &str,
    file_description: &str,
) {
    let ver = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "1.0.0".to_string());
    let file_ver = match ver.matches('.').count() {
        1 => format!("{ver}.0.0"),
        2 => format!("{ver}.0"),
        _ => ver.clone(),
    };

    res.set("FileDescription", file_description);
    res.set("ProductName", product_name);
    res.set("FileVersion", &file_ver);
    res.set("ProductVersion", &ver);
    res.set("CompanyName", AUTHOR);
    res.set("LegalCopyright", COPYRIGHT);
    res.set("OriginalFilename", original_filename);
}
