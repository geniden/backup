//! Windows: version and author metadata in exe file properties.

const AUTHOR: &str = "Emelyanov Anton";
const COPYRIGHT: &str =
    "Copyright (C) 2026 Emelyanov Anton (geniden@gmail.com). https://github.com/geniden/backup";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let mut res = winres::WindowsResource::new();
    apply_version_info(
        &mut res,
        "backup-server.exe",
        "Backup Server",
        "Backup Server",
    );
    res.compile().expect("failed to compile Windows resources");
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
