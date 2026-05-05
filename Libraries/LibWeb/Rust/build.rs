/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let libweb_dir = manifest_dir.parent().expect("Rust crate must live in LibWeb");
    let repository_root = libweb_dir
        .parent()
        .and_then(|libraries_dir| libraries_dir.parent())
        .expect("LibWeb must live under Libraries");
    let media_features_json = libweb_dir.join("CSS").join("MediaFeatures.json");
    let value_types_json = libweb_dir.join("CSS").join("ValueTypes.json");
    let media_features_generator = repository_root
        .join("Meta")
        .join("Generators")
        .join("generate_libweb_css_media_features_rust.py");
    let value_types_generator = repository_root
        .join("Meta")
        .join("Generators")
        .join("generate_libweb_css_value_types_rust.py");
    let generated_media_features = out_dir.join("generated_media_features.rs");
    let generated_value_types = out_dir.join("generated_value_types.rs");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-env-changed=FFI_OUTPUT_DIR");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed={}", media_features_json.display());
    println!("cargo:rerun-if-changed={}", value_types_json.display());
    println!("cargo:rerun-if-changed={}", media_features_generator.display());
    println!("cargo:rerun-if-changed={}", value_types_generator.display());

    let status = Command::new("python3")
        .arg(&media_features_generator)
        .arg("--json")
        .arg(&media_features_json)
        .arg("--output")
        .arg(&generated_media_features)
        .status()?;
    if !status.success() {
        return Err(format!("{} failed with status {status}", media_features_generator.display()).into());
    }

    let status = Command::new("python3")
        .arg(&value_types_generator)
        .arg("--json")
        .arg(&value_types_json)
        .arg("--output")
        .arg(&generated_value_types)
        .status()?;
    if !status.success() {
        return Err(format!("{} failed with status {status}", value_types_generator.display()).into());
    }

    let ffi_out_dir = env::var("FFI_OUTPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| out_dir.clone());

    cbindgen::generate(manifest_dir).map_or_else(
        |error| match error {
            cbindgen::Error::ParseSyntaxError { .. } => {}
            other => panic!("{other:?}"),
        },
        |bindings| {
            let header_path = out_dir.join("RustFFI.h");
            bindings.write_to_file(&header_path);

            if ffi_out_dir != out_dir {
                bindings.write_to_file(ffi_out_dir.join("RustFFI.h"));
            }
        },
    );

    Ok(())
}
