use crate::build_context::{BridgeModel, ProjectLayout};
use crate::BuildContext;
use crate::CargoToml;
use crate::Manylinux;
use crate::Metadata21;
use crate::PythonInterpreter;
use crate::Target;
use anyhow::{bail, format_err, Context, Result};
use cargo_metadata::{Metadata, MetadataCommand, Node};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use structopt::StructOpt;

/// High level API for building wheels from a crate which is also used for the CLI
#[derive(Debug, Serialize, Deserialize, StructOpt, Clone, Eq, PartialEq)]
#[serde(default)]
pub struct BuildOptions {
    // The {n} are workarounds for https://github.com/TeXitoi/structopt/issues/163
    /// Control the platform tag on linux.
    ///
    /// - `1`: Use the manylinux1 tag and check for compliance{n}
    /// - `1-unchecked`: Use the manylinux1 tag without checking for compliance{n}
    /// - `2010`: Use the manylinux2010 tag and check for compliance{n}
    /// - `2010-unchecked`: Use the manylinux2010 tag without checking for compliance{n}
    /// - `2014`: Use the manylinux2010 tag and check for compliance{n}
    /// - `2014-unchecked`: Use the manylinux2014 tag without checking for compliance{n}
    /// - `off`: Use the native linux tag (off)
    ///
    /// This option is ignored on all non-linux platforms
    #[structopt(
        long,
        possible_values = &["1", "1-unchecked", "2010", "2010-unchecked", "2014", "2014-unchecked", "off"],
        case_insensitive = true,
        default_value = "1"
    )]
    pub manylinux: Manylinux,
    #[structopt(short, long)]
    /// The python versions to build wheels for, given as the names of the
    /// interpreters. Uses autodiscovery if not explicitly set.
    pub interpreter: Option<Vec<PathBuf>>,
    /// Which kind of bindings to use. Possible values are pyo3, rust-cpython, cffi and bin
    #[structopt(short, long)]
    pub bindings: Option<String>,
    #[structopt(
        short = "m",
        long = "manifest-path",
        parse(from_os_str),
        default_value = "Cargo.toml",
        name = "PATH"
    )]
    /// The path to the Cargo.toml
    pub manifest_path: PathBuf,
    /// The directory to store the built wheels in. Defaults to a new "wheels"
    /// directory in the project's target directory
    #[structopt(short, long, parse(from_os_str))]
    pub out: Option<PathBuf>,
    /// Path to the Python source in mixed projects.
    #[structopt(long = "py-src", parse(from_os_str))]
    pub py_src: Option<PathBuf>,
    /// [deprecated, use --manylinux instead] Don't check for manylinux compliance
    #[structopt(long = "skip-auditwheel")]
    pub skip_auditwheel: bool,
    /// The --target option for cargo
    #[structopt(long, name = "TRIPLE")]
    pub target: Option<String>,
    /// Extra arguments that will be passed to cargo as `cargo rustc [...] [arg1] [arg2] --`
    ///
    /// Use as `--cargo-extra-args="--my-arg"`
    #[structopt(long = "cargo-extra-args")]
    pub cargo_extra_args: Vec<String>,
    /// Extra arguments that will be passed to rustc as `cargo rustc [...] -- [arg1] [arg2]`
    ///
    /// Use as `--rustc-extra-args="--my-arg"`
    #[structopt(long = "rustc-extra-args")]
    pub rustc_extra_args: Vec<String>,
}

impl Default for BuildOptions {
    fn default() -> Self {
        BuildOptions {
            manylinux: Manylinux::Manylinux1,
            interpreter: Some(vec![]),
            bindings: None,
            manifest_path: PathBuf::from("Cargo.toml"),
            py_src: None,
            out: None,
            skip_auditwheel: false,
            target: None,
            cargo_extra_args: Vec::new(),
            rustc_extra_args: Vec::new(),
        }
    }
}

impl BuildOptions {
    /// Tries to fill the missing metadata for a BuildContext by querying cargo and python
    pub fn into_build_context(self, release: bool, strip: bool) -> Result<BuildContext> {
        let manifest_file = self
            .manifest_path
            .canonicalize()
            .context(format_err!("Can't find {}", self.manifest_path.display()))?;

        if !manifest_file.is_file() {
            bail!(
                "{} (resolved to {}) is not the path to a Cargo.toml",
                self.manifest_path.display(),
                manifest_file.display()
            );
        };

        let cargo_toml = CargoToml::from_path(&manifest_file)?;
        let manifest_dir = manifest_file.parent().unwrap();
        let metadata21 = Metadata21::from_cargo_toml(&cargo_toml, &manifest_dir)
            .context("Failed to parse Cargo.toml into python metadata")?;
        let scripts = cargo_toml.scripts();

        // If the package name contains minuses, you must declare a module with
        // underscores as lib name
        let module_name = cargo_toml
            .lib
            .as_ref()
            .and_then(|lib| lib.name.as_ref())
            .unwrap_or_else(|| &cargo_toml.package.name)
            .to_owned();

        let project_layout = ProjectLayout::determine(manifest_dir, &module_name, self.py_src)?;

        let target = Target::from_target_triple(self.target.clone())?;

        let mut cargo_extra_args = split_extra_args(&self.cargo_extra_args)?;
        if let Some(target) = self.target {
            cargo_extra_args.extend_from_slice(&["--target".to_string(), target]);
        }

        let cargo_metadata_extra_args = extra_feature_args(&cargo_extra_args);

        let cargo_metadata = MetadataCommand::new()
            .manifest_path(&self.manifest_path)
            .other_options(cargo_metadata_extra_args)
            .exec()
            .context("Cargo metadata failed. Do you have cargo in your PATH?")?;

        let wheel_dir = match self.out {
            Some(ref dir) => dir.clone(),
            None => PathBuf::from(&cargo_metadata.target_directory).join("wheels"),
        };

        let bridge = find_bridge(&cargo_metadata, self.bindings.as_deref())?;

        if bridge != BridgeModel::Bin && module_name.contains('-') {
            bail!(
                "The module name must not contains a minus \
                 (Make sure you have set an appropriate [lib] name in your Cargo.toml)"
            );
        }

        let interpreter = match self.interpreter {
            // Only build a source ditribution
            Some(ref interpreter) if interpreter.is_empty() => vec![],
            // User given list of interpreters
            Some(interpreter) => find_interpreter(&bridge, &interpreter, &target)?,
            // Auto-detect interpreters
            None => find_interpreter(&bridge, &[], &target)?,
        };

        let rustc_extra_args = split_extra_args(&self.rustc_extra_args)?;

        let manylinux = if self.skip_auditwheel {
            eprintln!("⚠ --skip-auditwheel is deprecated, use --manylinux=1-unchecked");
            Manylinux::Manylinux1Unchecked
        } else {
            self.manylinux
        };

        Ok(BuildContext {
            target,
            bridge,
            project_layout,
            metadata21,
            scripts,
            module_name,
            manifest_path: self.manifest_path,
            out: wheel_dir,
            release,
            strip,
            manylinux,
            cargo_extra_args,
            rustc_extra_args,
            interpreter,
            cargo_metadata,
        })
    }
}

/// Tries to determine the [BridgeModel] for the target crate
pub fn find_bridge(cargo_metadata: &Metadata, bridge: Option<&str>) -> Result<BridgeModel> {
    let resolve = cargo_metadata
        .resolve
        .as_ref()
        .ok_or_else(|| format_err!("Expected to get a dependency graph from cargo"))?;

    let deps: HashMap<&str, &Node> = resolve
        .nodes
        .iter()
        .map(|node| (cargo_metadata[&node.id].name.as_ref(), node))
        .collect();

    if let Some(bindings) = bridge {
        if bindings == "cffi" {
            Ok(BridgeModel::Cffi)
        } else if bindings == "bin" {
            Ok(BridgeModel::Bin)
        } else {
            if !deps.contains_key(bindings) {
                bail!(
                    "The bindings crate {} was not found in the dependencies list",
                    bindings
                );
            }

            Ok(BridgeModel::Bindings(bindings.to_string()))
        }
    } else if let Some(node) = deps.get("pyo3") {
        println!("🔗 Found pyo3 bindings");
        if !node.features.contains(&"extension-module".to_string()) {
            let version = cargo_metadata[&node.id].version.to_string();
            println!(
                "⚠  Warning: You're building a library without activating pyo3's \
                 `extension-module` feature. \
                 See https://pyo3.rs/v{}/building_and_distribution.html#linking",
                version
            );
        }
        Ok(BridgeModel::Bindings("pyo3".to_string()))
    } else if deps.contains_key("cpython") {
        println!("🔗 Found rust-cpython bindings");
        Ok(BridgeModel::Bindings("rust_cpython".to_string()))
    } else {
        let package_id = resolve.root.as_ref().unwrap();
        let package = cargo_metadata
            .packages
            .iter()
            .find(|p| &p.id == package_id)
            .unwrap();

        if package.targets.len() == 1 {
            let target = &package.targets[0];
            if target
                .crate_types
                .iter()
                .any(|crate_type| crate_type == "cdylib")
            {
                return Ok(BridgeModel::Cffi);
            }
            if target
                .crate_types
                .iter()
                .any(|crate_type| crate_type == "bin")
            {
                return Ok(BridgeModel::Bin);
            }
        }
        bail!("Couldn't find any bindings; Please specify them with --bindings/-b")
    }
}

/// Finds the appropriate amount for python versions for each [BridgeModel].
///
/// This means all for bindings, one for cffi and zero for bin.
pub fn find_interpreter(
    bridge: &BridgeModel,
    interpreter: &[PathBuf],
    target: &Target,
) -> Result<Vec<PythonInterpreter>> {
    match bridge {
        BridgeModel::Bindings(_) => {
            let interpreter = if !interpreter.is_empty() {
                PythonInterpreter::check_executables(&interpreter, &target, &bridge)
                    .context("The given list of python interpreters is invalid")?
            } else {
                PythonInterpreter::find_all(&target, &bridge)
                    .context("Finding python interpreters failed")?
            };

            if interpreter.is_empty() {
                bail!("Couldn't find any python interpreters. Please specify at least one with -i");
            }

            println!(
                "🐍 Found {}",
                interpreter
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<String>>()
                    .join(", ")
            );

            Ok(interpreter)
        }
        BridgeModel::Cffi => {
            let executable = if interpreter.is_empty() {
                target.get_python()
            } else if interpreter.len() == 1 {
                interpreter[0].clone()
            } else {
                bail!("You can only specify one python interpreter for cffi compilation");
            };
            let err_message = "Failed to find python interpreter for generating cffi bindings";

            let interpreter = PythonInterpreter::check_executable(executable, &target, &bridge)
                .context(format_err!(err_message))?
                .ok_or_else(|| format_err!(err_message))?;

            println!("🐍 Using {} to generate the cffi bindings", interpreter);

            Ok(vec![interpreter])
        }
        BridgeModel::Bin => Ok(vec![]),
    }
}

/// Helper function that calls shlex on all extra args given
fn split_extra_args(given_args: &[String]) -> Result<Vec<String>> {
    let mut splitted_args = vec![];
    for arg in given_args {
        match shlex::split(&arg) {
            Some(split) => splitted_args.extend(split),
            None => {
                bail!(
                    "Couldn't split argument from `--cargo-extra-args`: '{}'",
                    arg
                );
            }
        }
    }
    Ok(splitted_args)
}

/// We need to pass feature flags to cargo metadata
/// (s. https://github.com/PyO3/maturin/issues/211), but we can't pass
/// all the extra args, as e.g. `--target` isn't supported.
/// So we try to extract all the arguments related to features and
/// hope that that's sufficient
fn extra_feature_args(cargo_extra_args: &[String]) -> Vec<String> {
    let mut cargo_metadata_extra_args = vec![];
    let mut feature_args = false;
    for arg in cargo_extra_args {
        if feature_args {
            if arg.starts_with('-') {
                feature_args = false;
            } else {
                cargo_metadata_extra_args.push(arg.clone());
            }
        }
        if arg == "--features" {
            cargo_metadata_extra_args.push(arg.clone());
            feature_args = true;
        } else if arg == "--all-features"
            || arg == "--no-default-features"
            || arg.starts_with("--features")
        {
            cargo_metadata_extra_args.push(arg.clone());
        }
    }
    cargo_metadata_extra_args
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;

    #[test]
    fn test_find_bridge_pyo3() {
        let pyo3_pure = MetadataCommand::new()
            .manifest_path(&Path::new("test-crates/pyo3-pure").join("Cargo.toml"))
            .exec()
            .unwrap();

        assert!(match find_bridge(&pyo3_pure, None).unwrap() {
            BridgeModel::Bindings(_) => true,
            _ => false,
        });

        assert!(match find_bridge(&pyo3_pure, Some("pyo3")).unwrap() {
            BridgeModel::Bindings(_) => true,
            _ => false,
        });

        assert!(find_bridge(&pyo3_pure, Some("rust-cpython")).is_err());
    }

    #[test]
    fn test_find_bridge_pyo3_feature() {
        let pyo3_pure = MetadataCommand::new()
            .manifest_path(&Path::new("test-crates/pyo3-feature").join("Cargo.toml"))
            .exec()
            .unwrap();

        assert!(find_bridge(&pyo3_pure, None).is_err());

        let pyo3_pure = MetadataCommand::new()
            .manifest_path(&Path::new("test-crates/pyo3-feature").join("Cargo.toml"))
            .other_options(vec!["--features=pyo3".to_string()])
            .exec()
            .unwrap();

        assert!(match find_bridge(&pyo3_pure, None).unwrap() {
            BridgeModel::Bindings(_) => true,
            _ => false,
        });
    }

    #[test]
    fn test_find_bridge_cffi() {
        let cffi_pure = MetadataCommand::new()
            .manifest_path(&Path::new("test-crates/cffi-pure").join("Cargo.toml"))
            .exec()
            .unwrap();

        assert_eq!(
            find_bridge(&cffi_pure, Some("cffi")).unwrap(),
            BridgeModel::Cffi
        );
        assert_eq!(find_bridge(&cffi_pure, None).unwrap(), BridgeModel::Cffi);

        assert!(find_bridge(&cffi_pure, Some("rust-cpython")).is_err());
        assert!(find_bridge(&cffi_pure, Some("pyo3")).is_err());
    }

    #[test]
    fn test_find_bridge_bin() {
        let hello_world = MetadataCommand::new()
            .manifest_path(&Path::new("test-crates/hello-world").join("Cargo.toml"))
            .exec()
            .unwrap();

        assert_eq!(
            find_bridge(&hello_world, Some("bin")).unwrap(),
            BridgeModel::Bin
        );
        assert_eq!(find_bridge(&hello_world, None).unwrap(), BridgeModel::Bin);

        assert!(find_bridge(&hello_world, Some("rust-cpython")).is_err());
        assert!(find_bridge(&hello_world, Some("pyo3")).is_err());
    }

    #[test]
    fn test_argument_splitting() {
        let mut options = BuildOptions::default();
        options.cargo_extra_args.push("--features log".to_string());
        options.bindings = Some("bin".to_string());
        let context = options.into_build_context(false, false).unwrap();
        assert_eq!(context.cargo_extra_args, vec!["--features", "log"])
    }

    #[test]
    fn test_extra_feature_args() {
        let cargo_extra_args = "--no-default-features --features a b --target x86_64-unknown-linux-musl --features=c --lib";
        let cargo_extra_args = split_extra_args(&[cargo_extra_args.to_string()]).unwrap();
        let cargo_metadata_extra_args = extra_feature_args(&cargo_extra_args);
        assert_eq!(
            cargo_metadata_extra_args,
            vec![
                "--no-default-features",
                "--features",
                "a",
                "b",
                "--features=c"
            ]
        );
    }
}
