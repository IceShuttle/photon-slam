use std::path::Path;
use std::process::Command;

fn main() {
    let shader_dir = Path::new("shaders");
    let slang_file = shader_dir.join("compute.slang");

    println!("cargo:rerun-if-changed={}", slang_file.display());

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let spv_output = Path::new(&out_dir).join("compute.spv");

    let status = Command::new("slangc")
        .args([
            "-target",
            "spirv",
            "-profile",
            "spirv_1_0",
            "-o",
            &spv_output.to_string_lossy(),
            &slang_file.to_string_lossy(),
        ])
        .status()
        .expect("failed to execute slangc — is shader-slang installed?");

    assert!(
        status.success(),
        "slangc compilation of shaders/compute.slang failed"
    );
    println!("Shaders Compiled");
}
