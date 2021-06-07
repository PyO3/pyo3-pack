use crate::build_context::BridgeModel;
use crate::compile::compile;
use crate::module_writer::{write_bindings_module, write_cffi_module, PathWriter};
use crate::PythonInterpreter;
use crate::Target;
use crate::{write_dist_info, BuildOptions};
use crate::{ModuleWriter, PlatformTag};
use anyhow::{anyhow, bail, format_err, Context, Result};
use fs_err as fs;
use std::path::Path;
use std::process::Command;

/// Installs a crate by compiling it and copying the shared library to site-packages.
/// Also adds the dist-info directory to make sure pip and other tools detect the library
///
/// Works only in a virtualenv.
pub fn develop(
    bindings: Option<String>,
    manifest_file: &Path,
    cargo_extra_args: Vec<String>,
    rustc_extra_args: Vec<String>,
    venv_dir: &Path,
    release: bool,
    strip: bool,
) -> Result<()> {
    let target = Target::from_target_triple(None)?;

    let python = target.get_venv_python(&venv_dir);

    let build_options = BuildOptions {
        platform_tag: Some(PlatformTag::Linux),
        interpreter: Some(vec![target.get_python()]),
        bindings,
        manifest_path: manifest_file.to_path_buf(),
        out: None,
        skip_auditwheel: false,
        target: None,
        cargo_extra_args,
        rustc_extra_args,
        universal2: false,
    };

    let build_context = build_options.into_build_context(release, strip)?;

    let interpreter = PythonInterpreter::check_executable(python, &target, &build_context.bridge)?
        .ok_or_else(|| {
            anyhow!("Expected `python` to be a python interpreter inside a virtualenv ಠ_ಠ")
        })?;

    // Install dependencies
    if !build_context.metadata22.requires_dist.is_empty() {
        let mut args = vec!["-m", "pip", "install"];
        args.extend(
            build_context
                .metadata22
                .requires_dist
                .iter()
                .map(|x| x.as_str()),
        );
        let status = Command::new(&interpreter.executable)
            .args(&args)
            .status()
            .context("Failed to run pip install")?;
        if !status.success() {
            bail!(r#"pip install finished with "{}""#, status)
        }
    }

    // First, uninstall the existing installation. Without this, the following can happen:
    // * `pip install my-project`: Creates files that maturin would not create, e.g. `my_project-1.0.0.dist-info/direct_url.json`
    // * `maturin develop`: Overwrites a RECORD file with one that doesn't list direct_url.json, while not removing the file
    // * `pip uninstall my-project`: Removes most things, but notes that direct_url.json won't be removed
    // * Any other pip action: Complains about my-project, crashes when trying to do anything with my-project
    //
    // Uninstalling the actual code is done individually for each bridge model
    let base_path = target.get_venv_site_package(venv_dir, &interpreter);
    let dist_info_dir = base_path.join(build_context.metadata22.get_dist_info_dir());
    if dist_info_dir.is_dir() {
        fs::remove_dir_all(&dist_info_dir).context(format!(
            "Failed to uninstall existing installation by removing {}",
            dist_info_dir.display()
        ))?;
    }

    let mut writer = PathWriter::venv(&target, &venv_dir, &build_context.bridge)?;

    let context = "Failed to build a native library through cargo";

    match build_context.bridge {
        BridgeModel::Bin => {
            let artifacts = compile(&build_context, None, &BridgeModel::Bin).context(context)?;

            let artifact = artifacts
                .get("bin")
                .ok_or_else(|| format_err!("Cargo didn't build a binary"))?;

            // Copy the artifact into the same folder as pip and python
            let bin_name = artifact.file_name().unwrap();
            let bin_path = target.get_venv_bin_dir(&venv_dir).join(bin_name);
            // No need to uninstall since we're overwriting anyway
            // TODO: Shouldn't this use a writer method so that it shows up in the RECORD?
            fs::copy(&artifact, &bin_path).context(format!(
                "Failed to copy {} to {}",
                artifact.display(),
                bin_path.display()
            ))?;
        }
        BridgeModel::Cffi => {
            let artifact = build_context.compile_cdylib(None, None).context(context)?;

            // Uninstall the old code
            writer.delete_dir(&build_context.module_name)?;

            write_cffi_module(
                &mut writer,
                &build_context.project_layout,
                &build_context.manifest_path.parent().unwrap(),
                &build_context.module_name,
                &artifact,
                &interpreter.executable,
                true,
            )?;
        }
        BridgeModel::Bindings(_) => {
            let artifact = build_context
                .compile_cdylib(Some(&interpreter), Some(&build_context.module_name))
                .context(context)?;

            // Uninstall the old code
            writer.delete_dir(&build_context.module_name)?;

            write_bindings_module(
                &mut writer,
                &build_context.project_layout,
                &build_context.module_name,
                &artifact,
                Some(&interpreter),
                &target,
                true,
            )?;
        }
        BridgeModel::BindingsAbi3(_, _) => {
            let artifact = build_context
                // We need the interpreter on windows
                .compile_cdylib(Some(&interpreter), Some(&build_context.module_name))
                .context(context)?;

            // Uninstall the old code
            writer.delete_dir(&build_context.module_name)?;

            write_bindings_module(
                &mut writer,
                &build_context.project_layout,
                &build_context.module_name,
                &artifact,
                None,
                &target,
                true,
            )?;
        }
    }

    // Write dist-info directory so pip can interact with it
    // We skip running auditwheel and simply tag as linux
    let tags = match build_context.bridge {
        BridgeModel::Bindings(_) => {
            vec![build_context.interpreter[0].get_tag(PlatformTag::Linux, build_context.universal2)]
        }
        BridgeModel::BindingsAbi3(major, minor) => {
            let platform = target.get_platform_tag(PlatformTag::Linux, build_context.universal2);
            vec![format!("cp{}{}-abi3-{}", major, minor, platform)]
        }
        BridgeModel::Bin | BridgeModel::Cffi => {
            build_context
                .target
                .get_universal_tags(PlatformTag::Linux, build_context.universal2)
                .1
        }
    };

    write_dist_info(&mut writer, &build_context.metadata22, &tags)?;

    // https://packaging.python.org/specifications/recording-installed-packages/#the-installer-file
    writer.add_bytes(
        build_context
            .metadata22
            .get_dist_info_dir()
            .join("INSTALLER"),
        env!("CARGO_PKG_NAME").as_bytes(),
    )?;

    writer.write_record(&build_context.metadata22)?;

    Ok(())
}
