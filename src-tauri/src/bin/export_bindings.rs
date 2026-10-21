fn main() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/bindings.ts");
    dae_lib::export_bindings(&path);
}
