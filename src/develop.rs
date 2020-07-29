use crate::build_context::BridgeModel;
use crate::compile::compile;
use crate::module_writer::{write_bindings_module, write_cffi_module, PathWriter};
use crate::BuildOptions;
use crate::Manylinux;
use crate::PythonInterpreter;
use crate::Target;
use anyhow::{anyhow, format_err, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Installs a crate by compiling it and copying the shared library to the right directory
///
/// Works only in a virtualenv.
#[allow(clippy::too_many_arguments)]
pub fn develop(
    bindings: Option<String>,
    manifest_file: &Path,
    cargo_extra_args: Vec<String>,
    rustc_extra_args: Vec<String>,
    venv_dir: &Path,
    release: bool,
    strip: bool,
    py_src: Option<PathBuf>,
) -> Result<()> {
    let target = Target::from_target_triple(None)?;

    let python = target.get_venv_python(&venv_dir);

    let build_options = BuildOptions {
        manylinux: Manylinux::Off,
        interpreter: Some(vec![target.get_python()]),
        bindings,
        manifest_path: manifest_file.to_path_buf(),
        out: None,
        skip_auditwheel: false,
        py_src,
        target: None,
        cargo_extra_args,
        rustc_extra_args,
    };

    let build_context = build_options.into_build_context(release, strip)?;

    let interpreter = PythonInterpreter::check_executable(python, &target, &build_context.bridge)?
        .ok_or_else(|| {
            anyhow!("Expected `python` to be a python interpreter inside a virtualenv ಠ_ಠ")
        })?;

    let mut builder = PathWriter::venv(&target, &venv_dir, &build_context.bridge)?;

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
            fs::copy(&artifact, &bin_path).context(format!(
                "Failed to copy {} to {}",
                artifact.display(),
                bin_path.display()
            ))?;
        }
        BridgeModel::Cffi => {
            let artifact = build_context.compile_cdylib(None, None).context(context)?;

            builder.delete_dir(&build_context.module_name)?;

            write_cffi_module(
                &mut builder,
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

            write_bindings_module(
                &mut builder,
                &build_context.project_layout,
                &build_context.module_name,
                &artifact,
                &interpreter,
                true,
            )?;
        }
    }

    Ok(())
}
