fn main() {
    // `generate_context!` (lib.rs) embeds the app icon when the crate is
    // compiled, and cargo doesn't know it reads icons/: without this, new
    // icons (`npx tauri icon …`) only show after an unrelated Rust change.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
