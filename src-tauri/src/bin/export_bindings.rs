fn main() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/bindings.ts");
    dae_lib::specta_builder()
        .export(specta_typescript::Typescript::default(), path)
        .expect("Failed to export TypeScript bindings");
}
