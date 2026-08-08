fn main() {
    dae_lib::specta_builder()
        .export(
            specta_typescript::Typescript::default(),
            "../src/bindings.ts",
        )
        .expect("Failed to export TypeScript bindings");
}
