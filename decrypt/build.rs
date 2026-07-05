//! Windows: build multi-size ICO from one master PNG and embed in backup-decrypt.exe.
//!
//! Master: `src/icons/backup-decrypt.ico` (256×256 PNG data).

use std::path::{Path, PathBuf};

use image::imageops::FilterType;
use image::GenericImageView;

const MASTER_ICON: &str = "src/icons/backup-decrypt.ico";
const ICON_SIZES: [u32; 5] = [32, 48, 64, 128, 256];
const AUTHOR: &str = "Emelyanov Anton";
const COPYRIGHT: &str =
    "Copyright (C) 2026 Emelyanov Anton (geniden@gmail.com). https://github.com/geniden/backup";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={MASTER_ICON}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let work_dir = out_dir.join("winres");
    std::fs::create_dir_all(&work_dir).expect("create winres work dir");

    let app_icon = work_dir.join("app.ico");
    merge_icons(&manifest_dir, &app_icon).expect("failed to build application icon");

    let icon_for_rc = app_icon.to_str().expect("app.ico path must be valid UTF-8");
    let mut res = winres::WindowsResource::new();
    res.set_icon(icon_for_rc);
    apply_version_info(
        &mut res,
        "backup-decrypt.exe",
        "Backup Decrypt",
        "Backup Decrypt",
    );

    let rc_path = work_dir.join("resource.rc");
    res.write_resource_file(&rc_path)
        .expect("failed to write resource.rc");

    embed_resource::compile_for(rc_path, &["backup-decrypt"], embed_resource::NONE);
}

fn merge_icons(manifest_dir: &Path, out_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let master_path = manifest_dir.join(MASTER_ICON);
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
