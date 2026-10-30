use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use reqwest::blocking::get;

fn main() {
    let arch =
        env::var("CARGO_CFG_TARGET_ARCH").expect("Cargo did not provide target architecture");
    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("Cargo did not provide target OS");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("Cargo did not provide OUT_DIR"));
    let dest_bin_path = out_dir.join("ttyd");

    fs::create_dir_all(&out_dir).expect("Failed to create output directory");
    if target_os == "macos" {
        // ttyd upstream only publishes Linux binaries. On macOS use the native
        // executable supplied by Homebrew (or explicitly through TTYD_BIN), so
        // the embedded terminal server has the same architecture as the agent.
        let source = macos_ttyd();
        fs::copy(&source, &dest_bin_path).unwrap_or_else(|error| {
            panic!(
                "Failed to copy macOS ttyd from {}: {error}",
                source.display()
            )
        });
        println!("cargo:rerun-if-env-changed=TTYD_BIN");
    } else if target_os == "linux" {
        let url = format!(
            "https://github.com/tsl0922/ttyd/releases/download/1.7.7/ttyd.{}",
            arch_to_file_arch(&arch)
        );
        let response =
            get(&url).unwrap_or_else(|error| panic!("Failed to download ttyd from {url}: {error}"));
        let response_data = response.bytes().expect("Failed to read ttyd download");
        fs::write(&dest_bin_path, response_data).expect("Failed to write ttyd binary");
    } else {
        panic!("Embedded ttyd is not supported for target OS: {target_os}");
    }

    // The embedded file is unpacked by the agent at runtime. Preserve execute
    // permissions for local macOS builds as well as downloaded Linux builds.
    #[cfg(unix)]
    {
        Command::new("chmod")
            .arg("+x")
            .arg(&dest_bin_path)
            .status()
            .expect("Failed to make ttyd executable");
    }

    println!("cargo:rustc-env=TTYD={}", dest_bin_path.display());
    println!("cargo:rerun-if-changed=build.rs");
}

fn macos_ttyd() -> PathBuf {
    if let Some(path) = env::var_os("TTYD_BIN") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return path;
        }
        panic!("TTYD_BIN does not name a file: {}", path.display());
    }

    let output = Command::new("which")
        .arg("ttyd")
        .output()
        .expect("Failed to search PATH for ttyd");
    if output.status.success() {
        let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        if path.is_file() {
            return path;
        }
    }

    panic!(
        "A native ttyd binary is required when building for macOS. Install it with `brew install ttyd`, or set TTYD_BIN=/path/to/ttyd"
    );
}

fn arch_to_file_arch(arch: &str) -> &str {
    match arch {
        "x86_64" => "i686",
        other => other,
    }
}
