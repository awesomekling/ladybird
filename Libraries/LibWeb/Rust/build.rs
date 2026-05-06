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
    let properties_json = libweb_dir.join("CSS").join("Properties.json");
    let pseudo_classes_json = libweb_dir.join("CSS").join("PseudoClasses.json");
    let pseudo_elements_json = libweb_dir.join("CSS").join("PseudoElements.json");
    let enums_json = libweb_dir.join("CSS").join("Enums.json");
    let logical_property_groups_json = libweb_dir.join("CSS").join("LogicalPropertyGroups.json");
    let units_json = libweb_dir.join("CSS").join("Units.json");
    let value_types_json = libweb_dir.join("CSS").join("ValueTypes.json");
    let transform_functions_json = libweb_dir.join("CSS").join("TransformFunctions.json");
    let media_features_generator = repository_root
        .join("Meta")
        .join("Generators")
        .join("generate_libweb_css_media_features_rust.py");
    let properties_generator = repository_root
        .join("Meta")
        .join("Generators")
        .join("generate_libweb_css_properties_rust.py");
    let pseudo_classes_generator = repository_root
        .join("Meta")
        .join("Generators")
        .join("generate_libweb_css_pseudo_class_rust.py");
    let pseudo_elements_generator = repository_root
        .join("Meta")
        .join("Generators")
        .join("generate_libweb_css_pseudo_element_rust.py");
    let units_generator = repository_root
        .join("Meta")
        .join("Generators")
        .join("generate_libweb_css_units_rust.py");
    let value_types_generator = repository_root
        .join("Meta")
        .join("Generators")
        .join("generate_libweb_css_value_types_rust.py");
    let transform_functions_generator = repository_root
        .join("Meta")
        .join("Generators")
        .join("generate_libweb_css_transform_functions_rust.py");
    let generated_media_features = out_dir.join("generated_media_features.rs");
    let generated_properties = out_dir.join("generated_properties.rs");
    let generated_pseudo_classes = out_dir.join("generated_pseudo_classes.rs");
    let generated_pseudo_elements = out_dir.join("generated_pseudo_elements.rs");
    let generated_units = out_dir.join("generated_units.rs");
    let generated_value_types = out_dir.join("generated_value_types.rs");
    let generated_transform_functions = out_dir.join("generated_transform_functions.rs");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    println!("cargo:rerun-if-env-changed=FFI_OUTPUT_DIR");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed={}", media_features_json.display());
    println!("cargo:rerun-if-changed={}", properties_json.display());
    println!("cargo:rerun-if-changed={}", pseudo_classes_json.display());
    println!("cargo:rerun-if-changed={}", pseudo_elements_json.display());
    println!("cargo:rerun-if-changed={}", enums_json.display());
    println!("cargo:rerun-if-changed={}", logical_property_groups_json.display());
    println!("cargo:rerun-if-changed={}", units_json.display());
    println!("cargo:rerun-if-changed={}", value_types_json.display());
    println!("cargo:rerun-if-changed={}", transform_functions_json.display());
    println!("cargo:rerun-if-changed={}", media_features_generator.display());
    println!("cargo:rerun-if-changed={}", properties_generator.display());
    println!("cargo:rerun-if-changed={}", pseudo_classes_generator.display());
    println!("cargo:rerun-if-changed={}", pseudo_elements_generator.display());
    println!("cargo:rerun-if-changed={}", units_generator.display());
    println!("cargo:rerun-if-changed={}", value_types_generator.display());
    println!("cargo:rerun-if-changed={}", transform_functions_generator.display());

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
        .arg(&properties_generator)
        .arg("--properties-json")
        .arg(&properties_json)
        .arg("--enums-json")
        .arg(&enums_json)
        .arg("--groups-json")
        .arg(&logical_property_groups_json)
        .arg("--output")
        .arg(&generated_properties)
        .status()?;
    if !status.success() {
        return Err(format!("{} failed with status {status}", properties_generator.display()).into());
    }

    let status = Command::new("python3")
        .arg(&pseudo_classes_generator)
        .arg("--json")
        .arg(&pseudo_classes_json)
        .arg("--output")
        .arg(&generated_pseudo_classes)
        .status()?;
    if !status.success() {
        return Err(format!("{} failed with status {status}", pseudo_classes_generator.display()).into());
    }

    let status = Command::new("python3")
        .arg(&pseudo_elements_generator)
        .arg("--json")
        .arg(&pseudo_elements_json)
        .arg("--output")
        .arg(&generated_pseudo_elements)
        .status()?;
    if !status.success() {
        return Err(format!("{} failed with status {status}", pseudo_elements_generator.display()).into());
    }

    let status = Command::new("python3")
        .arg(&units_generator)
        .arg("--json")
        .arg(&units_json)
        .arg("--output")
        .arg(&generated_units)
        .status()?;
    if !status.success() {
        return Err(format!("{} failed with status {status}", units_generator.display()).into());
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

    let status = Command::new("python3")
        .arg(&transform_functions_generator)
        .arg("--json")
        .arg(&transform_functions_json)
        .arg("--output")
        .arg(&generated_transform_functions)
        .status()?;
    if !status.success() {
        return Err(format!(
            "{} failed with status {status}",
            transform_functions_generator.display()
        )
        .into());
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
