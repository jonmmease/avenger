use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("avenger.pest");
    
    // Rerun if the pest file changes
    println!("cargo:rerun-if-changed=src/avenger.pest");
    
    // This ensures that the pest file is copied even if we're just building the library
    // (not necessarily running the binary)
    match fs::copy("src/avenger.pest", &dest_path) {
        Ok(_) => println!("Copied avenger.pest to {}", dest_path.display()),
        Err(e) => eprintln!("Error copying pest file: {}", e),
    }
} 