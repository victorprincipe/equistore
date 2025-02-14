use std::path::PathBuf;

fn main() {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    let generated_comment = "\
/* ============    Automatically generated file, DOT NOT EDIT.    ============ *
 *                                                                             *
 *    This file is automatically generated from the equistore sources,         *
 *    using cbindgen. If you want to make change to this file (including       *
 *    documentation), make the corresponding changes in the rust sources.      *
 * =========================================================================== */";

    let result = cbindgen::Builder::new()
        .with_crate(crate_dir)
        .with_config(cbindgen::Config {
            language: cbindgen::Language::C,
            cpp_compat: true,
            include_guard: Some("EQUISTORE_H".into()),
            include_version: false,
            documentation: true,
            documentation_style: cbindgen::DocumentationStyle::Doxy,
            header: Some(generated_comment.into()),
            ..Default::default()
        })
        .generate()
        .map(|data| {
            let mut path = PathBuf::from("include");
            path.push("equistore.h");
            data.write_to_file(&path);
        });

    // if not ok, rerun the build script unconditionally
    if result.is_ok() {
        println!("cargo:rerun-if-changed=src");
    }
}
