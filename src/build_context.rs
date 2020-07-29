#[cfg(feature = "auditwheel")]
use crate::auditwheel::auditwheel_rs;
use crate::compile;
use crate::compile::warn_missing_py_init;
use crate::module_writer::write_python_part;
use crate::module_writer::WheelWriter;
use crate::module_writer::{write_bin, write_bindings_module, write_cffi_module};
use crate::source_distribution::{get_pyproject_toml, source_distribution, warn_on_local_deps};
use crate::Manylinux;
use crate::Metadata21;
use crate::PythonInterpreter;
use crate::Target;
use anyhow::{anyhow, bail, Context, Result};
use cargo_metadata::Metadata;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The way the rust code is used in the wheel
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BridgeModel {
    /// A native module with c bindings, i.e. `#[no_mangle] extern "C" <some item>`
    Cffi,
    /// A rust binary to be shipped a python package
    Bin,
    /// A native module with pyo3 or rust-cpython bindings. The String is the name of the bindings
    /// providing crate, e.g. pyo3.
    Bindings(String),
}

impl BridgeModel {
    /// Returns the name of the bindings crate
    pub fn unwrap_bindings(&self) -> &str {
        match self {
            BridgeModel::Bindings(value) => &value,
            _ => panic!("Expected Bindings"),
        }
    }

    /// Test whether this is using a specific bindings crate
    pub fn is_bindings(&self, name: &str) -> bool {
        match self {
            BridgeModel::Bindings(value) => value == name,
            _ => false,
        }
    }
}

/// Whether this project is pure rust or rust mixed with python
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectLayout {
    /// A rust crate compiled into a shared library with only some glue python for cffi
    PureRust,
    /// A python package that is extended by a native rust module.
    ///
    /// Contains the canonicialized (i.e. absolute) path to the python part of the project
    Mixed(PathBuf),
}

impl ProjectLayout {
    /// Checks whether a python module exists besides Cargo.toml with the right name
    pub fn determine(
        project_root: impl AsRef<Path>,
        module_name: &str,
        py_src: Option<impl AsRef<Path>>,
    ) -> Result<ProjectLayout> {
        let python_package_dir = if let Some(py_src) = py_src {
            project_root
                .as_ref()
                .join(py_src.as_ref())
                .join(module_name)
        } else {
            project_root.as_ref().join(module_name)
        };
        if python_package_dir.is_dir() {
            if !python_package_dir.join("__init__.py").is_file() {
                bail!("Found a directory with the module name ({}) next to Cargo.toml, which indicates a mixed python/rust project, but the directory didn't contain an __init__.py file.", module_name)
            }

            println!("🍹 Building a mixed python/rust project");

            Ok(ProjectLayout::Mixed(python_package_dir))
        } else {
            Ok(ProjectLayout::PureRust)
        }
    }
}

/// Contains all the metadata required to build the crate
pub struct BuildContext {
    /// The platform, i.e. os and pointer width
    pub target: Target,
    /// Whether to use cffi or pyo3/rust-cpython
    pub bridge: BridgeModel,
    /// Whether this project is pure rust or rust mixed with python
    pub project_layout: ProjectLayout,
    /// Python Package Metadata 2.1
    pub metadata21: Metadata21,
    /// The `[console_scripts]` for the entry_points.txt
    pub scripts: HashMap<String, String>,
    /// The name of the module can be distinct from the package name, mostly
    /// because package names normally contain minuses while module names
    /// have underscores. The package name is part of metadata21
    pub module_name: String,
    /// The path to the Cargo.toml. Required for the cargo invocations
    pub manifest_path: PathBuf,
    /// The directory to store the built wheels in. Defaults to a new "wheels"
    /// directory in the project's target directory
    pub out: PathBuf,
    /// Pass --release to cargo
    pub release: bool,
    /// Strip the library for minimum file size
    pub strip: bool,
    /// Whether to use the the manylinux and check compliance (on), use it but don't
    /// check compliance (no-auditwheel) or use the native linux tag (off)
    pub manylinux: Manylinux,
    /// Extra arguments that will be passed to cargo as `cargo rustc [...] [arg1] [arg2] --`
    pub cargo_extra_args: Vec<String>,
    /// Extra arguments that will be passed to rustc as `cargo rustc [...] -- [arg1] [arg2]`
    pub rustc_extra_args: Vec<String>,
    /// The available python interpreter
    pub interpreter: Vec<PythonInterpreter>,
    /// Cargo.toml as resolved by [cargo_metadata]
    pub cargo_metadata: Metadata,
}

type BuiltWheelMetadata = (PathBuf, String, Option<PythonInterpreter>);

impl BuildContext {
    /// Checks which kind of bindings we have (pyo3/rust-cypthon or cffi or bin) and calls the
    /// correct builder. Returns a Vec that contains location, python tag (e.g. py3 or cp35)
    /// and for bindings the python interpreter they bind against.
    pub fn build_wheels(&self) -> Result<Vec<BuiltWheelMetadata>> {
        fs::create_dir_all(&self.out)
            .context("Failed to create the target directory for the wheels")?;

        let wheels = match &self.bridge {
            BridgeModel::Cffi => vec![(self.build_cffi_wheel()?, "py3".to_string(), None)],
            BridgeModel::Bin => vec![(self.build_bin_wheel()?, "py3".to_string(), None)],
            BridgeModel::Bindings(_) => self.build_binding_wheels()?,
        };

        Ok(wheels)
    }

    /// Builds a source distribution and returns the same metadata as [BuildContext::build_wheels]
    pub fn build_source_distribution(&self) -> Result<Option<BuiltWheelMetadata>> {
        fs::create_dir_all(&self.out)
            .context("Failed to create the target directory for the source distribution")?;

        match get_pyproject_toml(self.manifest_path.parent().unwrap()) {
            Ok(pyproject) => {
                warn_on_local_deps(&self.cargo_metadata);
                let sdist_path = source_distribution(
                    &self.out,
                    &self.metadata21,
                    &self.manifest_path,
                    pyproject.sdist_include(),
                )
                .context("Failed to build source distribution")?;
                Ok(Some((sdist_path, "source".to_string(), None)))
            }
            Err(_) => Ok(None),
        }
    }

    /// Builds wheels for a Cargo project for all given python versions.
    /// Return type is the same as [BuildContext::build_wheels()]
    ///
    /// Defaults to 3.{5, 6, 7, 8, 9} if no python versions are given
    /// and silently ignores all non-existent python versions.
    ///
    /// Runs [auditwheel_rs()] if not deactivated
    pub fn build_binding_wheels(
        &self,
    ) -> Result<Vec<(PathBuf, String, Option<PythonInterpreter>)>> {
        let mut wheels = Vec::new();
        for python_interpreter in &self.interpreter {
            let artifact =
                self.compile_cdylib(Some(&python_interpreter), Some(&self.module_name))?;

            let tag = python_interpreter.get_tag(&self.manylinux);

            let mut writer = WheelWriter::new(
                &tag,
                &self.out,
                &self.metadata21,
                &self.scripts,
                &[tag.clone()],
            )?;

            write_bindings_module(
                &mut writer,
                &self.project_layout,
                &self.module_name,
                &artifact,
                python_interpreter,
                false,
            )
            .context("Failed to add the files to the wheel")?;

            let wheel_path = writer.finish()?;

            println!(
                "📦 Built wheel for {} {}.{}{} to {}",
                python_interpreter.interpreter,
                python_interpreter.major,
                python_interpreter.minor,
                python_interpreter.abiflags,
                wheel_path.display()
            );

            wheels.push((
                wheel_path,
                format!("cp{}{}", python_interpreter.major, python_interpreter.minor),
                Some(python_interpreter.clone()),
            ));
        }

        Ok(wheels)
    }

    /// Runs cargo build, extracts the cdylib from the output, runs auditwheel and returns the
    /// artifact
    ///
    /// The module name is used to warn about missing a `PyInit_<module name>` function for
    /// bindings modules.
    pub fn compile_cdylib(
        &self,
        python_interpreter: Option<&PythonInterpreter>,
        module_name: Option<&str>,
    ) -> Result<PathBuf> {
        let artifacts = compile(&self, python_interpreter, &self.bridge)
            .context("Failed to build a native library through cargo")?;

        let artifact = artifacts.get("cdylib").cloned().ok_or_else(|| {
            anyhow!(
                "Cargo didn't build a cdylib. Did you miss crate-type = [\"cdylib\"] \
                 in the lib section of your Cargo.toml?",
            )
        })?;
        #[cfg(feature = "auditwheel")]
        {
            let target = python_interpreter
                .map(|x| &x.target)
                .unwrap_or(&self.target);

            auditwheel_rs(&artifact, target, &self.manylinux)
                .context("Failed to ensure manylinux compliance")?;
        }

        if let Some(module_name) = module_name {
            warn_missing_py_init(&artifact, module_name)
                .context("Failed to parse the native library")?;
        }

        Ok(artifact)
    }

    /// Builds a wheel with cffi bindings
    pub fn build_cffi_wheel(&self) -> Result<PathBuf> {
        let artifact = self.compile_cdylib(None, None)?;

        let (tag, tags) = self.target.get_universal_tags(&self.manylinux);

        let mut builder =
            WheelWriter::new(&tag, &self.out, &self.metadata21, &self.scripts, &tags)?;

        write_cffi_module(
            &mut builder,
            &self.project_layout,
            self.manifest_path.parent().unwrap(),
            &self.module_name,
            &artifact,
            &self.interpreter[0].executable,
            false,
        )?;

        let wheel_path = builder.finish()?;

        println!("📦 Built wheel to {}", wheel_path.display());

        Ok(wheel_path)
    }

    /// Builds a wheel that contains a binary
    ///
    /// Runs [auditwheel_rs()] if not deactivated
    pub fn build_bin_wheel(&self) -> Result<PathBuf> {
        let artifacts = compile(&self, None, &self.bridge)
            .context("Failed to build a native library through cargo")?;

        let artifact = artifacts
            .get("bin")
            .cloned()
            .ok_or_else(|| anyhow!("Cargo didn't build a binary"))?;

        #[cfg(feature = "auditwheel")]
        auditwheel_rs(&artifact, &self.target, &self.manylinux)
            .context("Failed to ensure manylinux compliance")?;

        let (tag, tags) = self.target.get_universal_tags(&self.manylinux);

        if !self.scripts.is_empty() {
            bail!("Defining entrypoints and working with a binary doesn't mix well");
        }

        let mut builder =
            WheelWriter::new(&tag, &self.out, &self.metadata21, &self.scripts, &tags)?;

        match self.project_layout {
            ProjectLayout::Mixed(ref python_module) => {
                write_python_part(&mut builder, python_module, &self.module_name)
                    .context("Failed to add the python module to the package")?;
            }
            ProjectLayout::PureRust => {}
        }

        // I wouldn't know of any case where this would be the wrong (and neither do
        // I know a better alternative)
        let bin_name = artifact
            .file_name()
            .expect("Couldn't get the filename from the binary produced by cargo");
        write_bin(&mut builder, &artifact, &self.metadata21, bin_name)?;

        let wheel_path = builder.finish()?;

        println!("📦 Built wheel to {}", wheel_path.display());

        Ok(wheel_path)
    }
}
