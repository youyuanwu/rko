use std::path::Path;

/// Generate the rko-sys source tree at `output_dir`.
///
/// 1. Resolves clang resource dir and kernel paths for libclang.
/// 2. Runs bnd-winmd on `rko.toml` to produce a `.winmd`.
/// 3. Runs `windows-bindgen --package` to emit `src/rko/*/mod.rs`.
/// 4. Saves the `.winmd` under `output_dir/winmd/`.
pub fn generate(output_dir: &Path) {
    let gen_dir = Path::new(env!("CARGO_MANIFEST_DIR"));

    // Step 1: Generate .winmd
    let winmd_dir = output_dir.join("winmd");
    std::fs::create_dir_all(&winmd_dir).expect("failed to create winmd directory");
    let winmd = winmd_dir.join("rko-sys.winmd");
    bnd_winmd::run(&gen_dir.join("rko.toml"), Some(&winmd))
        .expect("bnd-winmd failed to generate winmd");

    // Step 2: Generate crate source tree via windows-bindgen package mode
    windows_bindgen::bindgen([
        "--in",
        winmd.to_str().unwrap(),
        "--out",
        output_dir.to_str().unwrap(),
        "--filter",
        "rko",
        "--sys",
        "--package",
        "--no-toml",
    ])
    .unwrap();
}
