//! Startup banner (ASCII logo — customize text for BackupDecrypt).

/// Same template as backup-client; edit ASCII art here if needed.
pub const ASCII_LOGO: &str = r#"
 ____             _                _____                             _   
|  _ \           | |              |  __ \                           | |  
| |_) | __ _  ___| | ___   _ _ __ | |  | | ___  ___ _ __ _   _ _ __ | |_ 
|  _ < / _` |/ __| |/ / | | | '_ \| |  | |/ _ \/ __| '__| | | | '_ \| __|
| |_) | (_| | (__|   <| |_| | |_) | |__| |  __/ (__| |  | |_| | |_) | |_ 
|____/ \__,_|\___|_|\_\\__,_| .__/|_____/ \___|\___|_|   \__, | .__/ \__|
                            | |                           __/ | |        
                            |_|                          |___/|_|                                     
"#;

pub const SUBTITLE: &str = "AES backups (.zip.aes / .txt.aes)";
pub const OFFICIAL_WEBSITE: &str = "https://github.com/geniden/backup";

pub fn print_logo() {
    println!();
    print!("{ASCII_LOGO}");
    println!("BACKUP DECRYPT v{}", env!("CARGO_PKG_VERSION"));
    println!("{SUBTITLE}");
    println!("{OFFICIAL_WEBSITE}");
    println!("{}", "─".repeat(60));
}
