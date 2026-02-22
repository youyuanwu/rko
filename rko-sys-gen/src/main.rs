use std::path::PathBuf;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let workspace_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let rko_sys_dir = workspace_dir.join("rko-sys");

    rko_sys_gen::generate(&rko_sys_dir);

    println!("Generated rko-sys crate at {}", rko_sys_dir.display());
}
