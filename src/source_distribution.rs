use crate::module_writer::ModuleWriter;
use crate::{Metadata22, SDistWriter};
use anyhow::{bail, Context, Result};
use cargo_metadata::Metadata;
use fs_err as fs;
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str;

const LOCAL_DEPENDENCIES_FOLDER: &str = "local_dependencies";

/// We need cargo to load the local dependencies from the location where we put them in the source
/// distribution. Since there is no cargo-backed way to replace dependencies
/// (see https://github.com/rust-lang/cargo/issues/9170), we do a simple
/// Cargo.toml rewrite ourselves.
/// A big chunk of that (including toml_edit) comes from cargo edit, and esp.
/// https://github.com/killercup/cargo-edit/blob/2a08f0311bcb61690d71d39cb9e55e69b256c8e1/src/manifest.rs
/// This method is rather frail, but unfortunately I don't know a better solution.
fn rewrite_cargo_toml(
    manifest_path: impl AsRef<Path>,
    known_path_deps: &HashMap<String, String>,
    root_crate: bool,
) -> Result<String> {
    let text = fs::read_to_string(&manifest_path).context(format!(
        "Can't read Cargo.toml at {}",
        manifest_path.as_ref().display(),
    ))?;
    let mut data = text.parse::<toml_edit::Document>().context(format!(
        "Failed to parse Cargo.toml at {}",
        manifest_path.as_ref().display()
    ))?;
    //  ˇˇˇˇˇˇˇˇˇˇˇˇ dep_category
    // [dependencies]
    // some_path_dep = { path = "../some_path_dep" }
    //                          ^^^^^^^^^^^^^^^^^^ table[&dep_name]["path"]
    // ^^^^^^^^^^^^^ dep_name
    for dep_category in &["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(table) = data[&dep_category].as_table_mut() {
            let dep_names: Vec<_> = table.iter().map(|(key, _)| key.to_string()).collect();
            for dep_name in dep_names {
                // There should either be no value for path, or it should be a string
                if table[&dep_name]["path"].is_none() {
                    continue;
                }
                if !table[&dep_name]["path"].is_str() {
                    bail!(
                        "In {}, {} {} has a path value that is not a string",
                        manifest_path.as_ref().display(),
                        dep_category,
                        dep_name
                    )
                }
                // This is the location of the targeted crate in the source distribution
                table[&dep_name]["path"] = if root_crate {
                    toml_edit::value(format!("{}/{}", LOCAL_DEPENDENCIES_FOLDER, dep_name))
                } else {
                    // Cargo.toml contains relative paths, and we're already in LOCAL_DEPENDENCIES_FOLDER
                    toml_edit::value(format!("../{}", dep_name))
                };
                if !known_path_deps.contains_key(&dep_name) {
                    bail!(
                        "cargo metadata does not know about the path for {}.{} present in {}, \
                        which should never happen ಠ_ಠ",
                        dep_category,
                        dep_name,
                        manifest_path.as_ref().display()
                    );
                }
            }
        }
    }
    Ok(data.to_string_in_original_order())
}

/// Copies the files of a crate to a source distribution, recursively adding path dependencies
/// and rewriting path entries in Cargo.toml
///
/// Runs `cargo package --list --allow-dirty` to obtain a list of files to package.
fn add_crate_to_source_distribution(
    writer: &mut SDistWriter,
    manifest_path: impl AsRef<Path>,
    prefix: impl AsRef<Path>,
    known_path_deps: &HashMap<String, String>,
    root_crate: bool,
) -> Result<()> {
    let output = Command::new("cargo")
        .args(&["package", "--list", "--allow-dirty", "--manifest-path"])
        .arg(manifest_path.as_ref())
        .output()
        .context("Failed to run cargo")?;
    if !output.status.success() {
        bail!(
            "Failed to query file list from cargo: {}\n--- Stdout:\n{}\n--- Stderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    let file_list: Vec<&Path> = str::from_utf8(&output.stdout)
        .context("Cargo printed invalid utf-8 ಠ_ಠ")?
        .lines()
        .map(Path::new)
        .collect();

    let manifest_dir = manifest_path.as_ref().parent().unwrap();

    let target_source: Vec<(PathBuf, PathBuf)> = file_list
        .iter()
        .map(|relative_to_manifests| {
            let relative_to_cwd = manifest_dir.join(relative_to_manifests);
            (relative_to_manifests.to_path_buf(), relative_to_cwd)
        })
        // We rewrite Cargo.toml and add it separately
        .filter(|(target, source)| {
            // Skip generated files. See https://github.com/rust-lang/cargo/issues/7938#issuecomment-593280660
            // and https://github.com/PyO3/maturin/issues/449
            if target == Path::new("Cargo.toml.orig") || target == Path::new("Cargo.toml") {
                false
            } else if target == Path::new(".cargo_vcs_info.json")
                || target == Path::new("Cargo.lock")
            {
                source.exists()
            } else {
                true
            }
        })
        .collect();

    if root_crate
        && !target_source
            .iter()
            .any(|(target, _)| target == Path::new("pyproject.toml"))
    {
        bail!(
            "pyproject.toml was not included by `cargo package`. \
                 Please make sure pyproject.toml is not excluded or build with `--no-sdist`"
        )
    }

    let rewritten_cargo_toml = rewrite_cargo_toml(&manifest_path, &known_path_deps, root_crate)?;

    writer.add_directory(&prefix)?;
    writer.add_bytes(
        prefix
            .as_ref()
            .join(manifest_path.as_ref().file_name().unwrap()),
        rewritten_cargo_toml.as_bytes(),
    )?;
    for (target, source) in target_source {
        writer.add_file(prefix.as_ref().join(target), source)?;
    }

    Ok(())
}

/// Creates aif source distribution, packing the root crate and all local dependencies
///
/// The source distribution format is specified in
/// [PEP 517 under "build_sdist"](https://www.python.org/dev/peps/pep-0517/#build-sdist)
/// and in
/// https://packaging.python.org/specifications/source-distribution-format/#source-distribution-file-format
pub fn source_distribution(
    wheel_dir: impl AsRef<Path>,
    metadata22: &Metadata22,
    manifest_path: impl AsRef<Path>,
    cargo_metadata: &Metadata,
    sdist_include: Option<&Vec<String>>,
) -> Result<PathBuf> {
    // Parse ids in the format:
    // some_path_dep 0.1.0 (path+file:///home/konsti/maturin/test-crates/some_path_dep)
    // This is not a good way to identify path dependencies, but I don't know a better one
    let matcher = Regex::new(r"^(.*) .* \(path\+file://(.*)\)$").unwrap();
    let resolve = cargo_metadata
        .resolve
        .as_ref()
        .context("Expected to get a dependency graph from cargo")?;
    let known_path_deps: HashMap<String, String> = resolve
        .nodes
        .iter()
        .filter(|node| &node.id != resolve.root.as_ref().unwrap())
        .filter_map(|node| matcher.captures(&node.id.repr))
        .map(|captures| (captures[1].to_string(), captures[2].to_string()))
        .collect();

    let mut writer = SDistWriter::new(wheel_dir, &metadata22)?;
    let root_dir = PathBuf::from(format!(
        "{}-{}",
        &metadata22.get_distribution_escaped(),
        &metadata22.get_version_escaped()
    ));

    // Add local path dependencies
    for (name, path) in known_path_deps.iter() {
        add_crate_to_source_distribution(
            &mut writer,
            &PathBuf::from(path).join("Cargo.toml"),
            &root_dir.join(LOCAL_DEPENDENCIES_FOLDER).join(name),
            &known_path_deps,
            false,
        )
        .context(format!(
            "Failed to add local dependency {} at {} to the source distribution",
            name, path
        ))?;
    }

    // Add the main crate
    add_crate_to_source_distribution(
        &mut writer,
        &manifest_path,
        &root_dir,
        &known_path_deps,
        true,
    )?;

    let manifest_dir = manifest_path.as_ref().parent().unwrap();

    if let Some(include_targets) = sdist_include {
        for pattern in include_targets {
            println!("📦 Including files matching \"{}\"", pattern);
            for source in glob::glob(&manifest_dir.join(pattern).to_string_lossy())
                .expect("No files found for pattern")
                .filter_map(Result::ok)
            {
                let target = root_dir.join(&source.strip_prefix(manifest_dir)?);
                writer.add_file(target, source)?;
            }
        }
    }

    writer.add_bytes(
        root_dir.join("PKG-INFO"),
        metadata22.to_file_contents().as_bytes(),
    )?;

    let source_distribution_path = writer.finish()?;

    println!(
        "📦 Built source distribution to {}",
        source_distribution_path.display()
    );

    Ok(source_distribution_path)
}
