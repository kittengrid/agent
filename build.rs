use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use reqwest::blocking::get;

fn main() {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Define the download URLs for each binary
    let binaries = [(
        "ttyd",
        format!(
            "https://github.com/tsl0922/ttyd/releases/download/1.7.7/ttyd.{}",
            arch_to_file_arch(&arch)
        ),
    )];

    for (bin_name, url) in binaries.iter() {
        let response = get(url).expect("Failed to download file");
        let response_data = response.bytes().expect("Failed to read file");
        let dest_bin_path = out_dir.join(bin_name);

        // write the binary to a temporary file
        fs::create_dir_all(&out_dir).expect("Failed to create output directory");
        fs::write(&dest_bin_path, &response_data).expect("Failed to write binary");

        // Ensure it's executable (only needed on Unix)
        #[cfg(unix)]
        {
            Command::new("chmod")
                .arg("+x")
                .arg(&dest_bin_path)
                .status()
                .expect("Failed to make binary executable");
        }

        // Set environment variables to reference the extracted binary
        println!(
            "cargo:rustc-env={}={}",
            bin_name.to_uppercase(),
            dest_bin_path.display()
        );
    }

    println!("cargo:rerun-if-changed=build.rs");
}

fn arch_to_file_arch(arch: &str) -> &str {
    match arch {
        "x86_64" => "i686",
        other => other,
    }
}
