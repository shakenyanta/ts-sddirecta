fn main() {
    // The generated context reads icons through a macro cache. Track the source
    // files too so icon-only updates rebuild both the context and Windows resource.
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
