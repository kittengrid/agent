use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;

pub fn install_binary(binary_bytes: &[u8]) -> String {
    // Get a temporary file path
    let temp_dir = env::temp_dir();

    // random file name
    let bin_path = temp_dir.join(format!("agent_bin_{}", rand::random::<u32>()));

    // Write bytes to file
    fs::write(&bin_path, binary_bytes).expect("Failed to write binary file");

    let mut perms = fs::metadata(&bin_path)
        .expect("Failed to read metadata")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&bin_path, perms).expect("Failed to set execute permissions");
    bin_path.to_str().unwrap().to_string()
}
