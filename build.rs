use std::path::Path;
use std::process::Command;

fn main() {
    // Re-run when shaders/ directory changes (new files added/removed).
    println!("cargo:rerun-if-changed=shaders/");
    let shader_dir = Path::new("shaders");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir);

    // Compile every entry-point .slang in the flat shaders/ directory.
    // math.slang is a module (imported, not an entry point) — skip it.
    if let Ok(entries) = std::fs::read_dir(shader_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "slang") && path.file_stem().unwrap() != "math"
            {
                let stem = path.file_stem().unwrap().to_str().unwrap();
                compile_shader(&path, &out_path.join(format!("{stem}.spv")));
            }
        }
    }
}

fn compile_shader(src: &Path, dst: &Path) {
    println!("cargo:rerun-if-changed={}", src.display());

    let status = Command::new("slangc")
        .args([
            "-target",
            "spirv",
            "-profile",
            "spirv_1_0",
            "-I",
            "shaders", // so `#include "math.slang"` resolves
            "-o",
            &dst.to_string_lossy(),
            &src.to_string_lossy(),
        ])
        .status()
        .expect("failed to execute slangc — is shader-slang installed?");

    assert!(
        status.success(),
        "slangc compilation of {} failed",
        src.display()
    );
}
