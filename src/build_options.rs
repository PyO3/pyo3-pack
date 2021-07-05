use crate::auditwheel::PlatformTag;
use crate::build_context::{BridgeModel, ProjectLayout};
use crate::cross_compile::{find_sysconfigdata, is_cross_compiling, parse_sysconfigdata};
use crate::python_interpreter::InterpreterKind;
use crate::BuildContext;
use crate::CargoToml;
use crate::Metadata21;
use crate::PythonInterpreter;
use crate::Target;
use anyhow::{bail, format_err, Context, Result};
use cargo_metadata::{Metadata, MetadataCommand, Node};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::io;
use std::path::PathBuf;
use structopt::StructOpt;

/// High level API for building wheels from a crate which is also used for the CLI
#[derive(Debug, Serialize, Deserialize, StructOpt, Clone, Eq, PartialEq)]
#[serde(default)]
pub struct BuildOptions {
    /// Control the platform tag on linux.
    ///
    /// Options are `manylinux` tags (for example `manylinux2014`/`manylinux_2_24`)
    /// or `musllinux` tags (for example `musllinux_1_2`)
    /// and `linux` for the native linux tag.
    ///
    /// Note that `manylinux1` is unsupported by the rust compiler. Wheels with the native `linux` tag
    /// will be rejected by pypi, unless they are separately validated by `auditwheel`.
    ///
    /// The default is the lowest compatible `manylinux` tag, or plain `linux` if nothing matched
    ///
    /// This option is ignored on all non-linux platforms
    #[structopt(
        name = "compatibility",
        long = "compatibility",
        alias = "manylinux",
        parse(try_from_str)
    )]
    pub platform_tag: Option<PlatformTag>,
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
    /// Don't check for manylinux compliance
    #[structopt(long = "skip-auditwheel")]
    pub skip_auditwheel: bool,
    /// The --target option for cargo
    #[structopt(long, name = "TRIPLE", env = "CARGO_BUILD_TARGET")]
    pub target: Option<String>,
    /// Extra arguments that will be passed to cargo as `cargo rustc [...] [arg1] [arg2] -- [...]`
    ///
    /// Use as `--cargo-extra-args="--my-arg"`
    ///
    /// Note that maturin invokes cargo twice: Once as `cargo metadata` and then as `cargo rustc`.
    /// maturin tries to pass only the shared subset of options to cargo metadata, but this is may
    /// be a bit flaky.
    #[structopt(long = "cargo-extra-args")]
    pub cargo_extra_args: Vec<String>,
    /// Extra arguments that will be passed to rustc as `cargo rustc [...] -- [...] [arg1] [arg2]`
    ///
    /// Use as `--rustc-extra-args="--my-arg"`
    #[structopt(long = "rustc-extra-args")]
    pub rustc_extra_args: Vec<String>,
    /// Control whether to build universal2 wheel for macOS or not.
    /// Only applies to macOS targets, do nothing otherwise.
    #[structopt(long)]
    pub universal2: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        BuildOptions {
            platform_tag: None,
            interpreter: Some(vec![]),
            bindings: None,
            manifest_path: PathBuf::from("Cargo.toml"),
            out: None,
            skip_auditwheel: false,
            target: None,
            cargo_extra_args: Vec::new(),
            rustc_extra_args: Vec::new(),
            universal2: false,
        }
    }
}

impl BuildOptions {
    /// Tries to fill the missing metadata for a BuildContext by querying cargo and python
    pub fn into_build_context(self, release: bool, strip: bool) -> Result<BuildContext> {
        if self.platform_tag == Some(PlatformTag::manylinux1()) {
            eprintln!("⚠  Warning: manylinux1 is unsupported by the Rust compiler.");
        }
        let manifest_file = &self.manifest_path;
        if !manifest_file.exists() {
            let current_dir =
                env::current_dir().context("Failed to detect current directory ಠ_ಠ")?;
            bail!(
                "Can't find {} (in {})",
                self.manifest_path.display(),
                current_dir.display()
            );
        }

        if !manifest_file.is_file() {
            bail!(
                "{} (resolved to {}) is not the path to a Cargo.toml",
                self.manifest_path.display(),
                manifest_file.display()
            );
        }

        let cargo_toml = CargoToml::from_path(&manifest_file)?;
        let manifest_dir = manifest_file.parent().unwrap();
        let metadata21 = Metadata21::from_cargo_toml(&cargo_toml, &manifest_dir)
            .context("Failed to parse Cargo.toml into python metadata")?;
        let extra_metadata = cargo_toml.remaining_core_metadata();

        let crate_name = &cargo_toml.package.name;

        // If the package name contains minuses, you must declare a module with
        // underscores as lib name
        let module_name = cargo_toml
            .lib
            .as_ref()
            .and_then(|lib| lib.name.as_ref())
            .unwrap_or(&crate_name)
            .to_owned();

        // Only use extension name from extra metadata if it contains dot
        let extension_name = extra_metadata
            .name
            .as_ref()
            .filter(|name| name.contains('.'))
            .unwrap_or(&module_name);

        let py_src = extra_metadata.python_source.clone();
        let project_layout = ProjectLayout::determine(manifest_dir, &extension_name, py_src)?;

        let mut cargo_extra_args = split_extra_args(&self.cargo_extra_args)?;
        if let Some(ref target) = self.target {
            cargo_extra_args.extend(vec!["--target".to_string(), target.clone()]);
        }

        let cargo_metadata_extra_args = extract_cargo_metadata_args(&cargo_extra_args)?;

        let result = MetadataCommand::new()
            .manifest_path(&self.manifest_path)
            .other_options(cargo_metadata_extra_args)
            .exec();

        let cargo_metadata = match result {
            Ok(cargo_metadata) => cargo_metadata,
            Err(cargo_metadata::Error::Io(inner)) if inner.kind() == io::ErrorKind::NotFound => {
                // NotFound is the specific error when cargo is not in PATH
                return Err(inner)
                    .context("Cargo metadata failed. Do you have cargo in your PATH?");
            }
            Err(err) => {
                return Err(err)
                    .context("Cargo metadata failed. Does your crate compile with `cargo build`?");
            }
        };

        let bridge = find_bridge(&cargo_metadata, self.bindings.as_deref())?;

        if bridge != BridgeModel::Bin && module_name.contains('-') {
            bail!(
                "The module name must not contains a minus \
                 (Make sure you have set an appropriate [lib] name in your Cargo.toml)"
            );
        }

        let target = Target::from_target_triple(self.target.clone())?;

        let wheel_dir = match self.out {
            Some(ref dir) => dir.clone(),
            None => PathBuf::from(&cargo_metadata.target_directory).join("wheels"),
        };

        let interpreter = match self.interpreter {
            // Only build a source distribution
            Some(ref interpreter) if interpreter.is_empty() => vec![],
            // User given list of interpreters
            Some(interpreter) => find_interpreter(&bridge, &interpreter, &target, None)?,
            // Auto-detect interpreters
            None => find_interpreter(&bridge, &[], &target, get_min_python_minor(&metadata21))?,
        };

        let rustc_extra_args = split_extra_args(&self.rustc_extra_args)?;

        let mut universal2 = self.universal2;
        // Also try to determine universal2 from ARCHFLAGS environment variable
        if let Ok(arch_flags) = env::var("ARCHFLAGS") {
            let arches: HashSet<&str> = arch_flags
                .split("-arch")
                .filter_map(|x| {
                    let x = x.trim();
                    if x.is_empty() {
                        None
                    } else {
                        Some(x)
                    }
                })
                .collect();
            if arches.contains("x86_64") && arches.contains("arm64") {
                universal2 = true;
            }
        };

        Ok(BuildContext {
            target,
            bridge,
            project_layout,
            metadata21,
            crate_name: crate_name.to_string(),
            module_name,
            manifest_path: self.manifest_path,
            out: wheel_dir,
            release,
            strip,
            skip_auditwheel: self.skip_auditwheel,
            platform_tag: self.platform_tag,
            cargo_extra_args,
            rustc_extra_args,
            interpreter,
            cargo_metadata,
            universal2,
        })
    }
}

/// Uses very simple PEP 440 subset parsing to determine the
/// minimum supported python minor version for interpreter search
fn get_min_python_minor(metadata21: &Metadata21) -> Option<usize> {
    if let Some(requires_python) = &metadata21.requires_python {
        let regex = Regex::new(r#">=3\.(\d+)(?:\.\d)?"#).unwrap();
        if let Some(captures) = regex.captures(&requires_python) {
            let min_python_minor = captures[1]
                .parse::<usize>()
                .expect("Regex must only match usize");
            Some(min_python_minor)
        } else {
            println!(
                "⚠  Couldn't parse the value of requires-python, \
                    not taking it into account when searching for python interpreter. \
                    Note: Only `>=3.x.y` is currently supported."
            );
            None
        }
    } else {
        None
    }
}

/// pyo3 supports building abi3 wheels if the unstable-api feature is not selected
fn has_abi3(cargo_metadata: &Metadata) -> Result<Option<(u8, u8)>> {
    let resolve = cargo_metadata
        .resolve
        .as_ref()
        .context("Expected cargo to return metadata with resolve")?;
    let pyo3_packages = resolve
        .nodes
        .iter()
        .filter(|package| cargo_metadata[&package.id].name == "pyo3")
        .collect::<Vec<_>>();
    match pyo3_packages.as_slice() {
        [pyo3_crate] => {
            // Find the minimal abi3 python version. If there is none, abi3 hasn't been selected
            // This parser abi3-py{major}{minor} and returns the minimal (major, minor) tuple
            let abi3_selected = pyo3_crate.features.iter().any(|x| x == "abi3");

            let min_abi3_version = pyo3_crate
                .features
                .iter()
                .filter(|x| x.starts_with("abi3-py") && x.len() == "abi3-pyxx".len())
                .map(|x| {
                    Ok((
                        (x.as_bytes()[7] as char).to_string().parse::<u8>()?,
                        (x.as_bytes()[8] as char).to_string().parse::<u8>()?,
                    ))
                })
                .collect::<Result<Vec<(u8, u8)>>>()
                .context("Bogus pyo3 cargo features")?
                .into_iter()
                .min();
            if abi3_selected && min_abi3_version.is_none() {
                bail!(
                    "You have selected the `abi3` feature but not a minimum version (e.g. the `abi3-py36` feature). \
                    maturin needs a minimum version feature to build abi3 wheels."
                )
            }
            Ok(min_abi3_version)
        }
        _ => bail!(format!(
            "Expected exactly one pyo3 dependency, found {}",
            pyo3_packages.len()
        )),
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

    let bridge = if let Some(bindings) = bridge {
        if bindings == "cffi" {
            BridgeModel::Cffi
        } else if bindings == "bin" {
            println!("🔗 Found bin bindings");
            BridgeModel::Bin
        } else {
            if !deps.contains_key(bindings) {
                bail!(
                    "The bindings crate {} was not found in the dependencies list",
                    bindings
                );
            }

            BridgeModel::Bindings(bindings.to_string())
        }
    } else if deps.get("pyo3").is_some() {
        BridgeModel::Bindings("pyo3".to_string())
    } else if deps.contains_key("cpython") {
        println!("🔗 Found rust-cpython bindings");
        BridgeModel::Bindings("rust_cpython".to_string())
    } else {
        let package = &cargo_metadata[resolve.root.as_ref().unwrap()];
        let targets: Vec<_> = package
            .targets
            .iter()
            .map(|target| target.crate_types.iter())
            .flatten()
            .map(String::as_str)
            .collect();

        if targets.contains(&"cdylib") {
            BridgeModel::Cffi
        } else if targets.contains(&"bin") {
            BridgeModel::Bin
        } else {
            bail!("Couldn't detect the binding type; Please specify them with --bindings/-b")
        }
    };

    if BridgeModel::Bindings("pyo3".to_string()) == bridge {
        let pyo3_node = deps["pyo3"];
        if !pyo3_node.features.contains(&"extension-module".to_string()) {
            let version = cargo_metadata[&pyo3_node.id].version.to_string();
            println!(
                "⚠  Warning: You're building a library without activating pyo3's \
                 `extension-module` feature. \
                 See https://pyo3.rs/v{}/building_and_distribution.html#linking",
                version
            );
        }

        if let Some((major, minor)) = has_abi3(&cargo_metadata)? {
            println!(
                "🔗 Found pyo3 bindings with abi3 support for Python ≥ {}.{}",
                major, minor
            );
            return Ok(BridgeModel::BindingsAbi3(major, minor));
        } else {
            println!("🔗 Found pyo3 bindings");
            return Ok(bridge);
        }
    }

    Ok(bridge)
}

/// Shared between cffi and pyo3-abi3
fn find_single_python_interpreter(
    bridge: &BridgeModel,
    interpreter: &[PathBuf],
    target: &Target,
    bridge_name: &str,
) -> Result<PythonInterpreter> {
    let err_message = "Failed to find a python interpreter";

    let executable = if interpreter.is_empty() {
        target.get_python()
    } else if interpreter.len() == 1 {
        interpreter[0].clone()
    } else {
        bail!(
            "You can only specify one python interpreter for {}",
            bridge_name
        );
    };

    let interpreter = PythonInterpreter::check_executable(executable, &target, &bridge)
        .context(format_err!(err_message))?
        .ok_or_else(|| format_err!(err_message))?;
    Ok(interpreter)
}

/// Finds the appropriate amount for python versions for each [BridgeModel].
///
/// This means all for bindings, one for cffi and zero for bin.
pub fn find_interpreter(
    bridge: &BridgeModel,
    interpreter: &[PathBuf],
    target: &Target,
    min_python_minor: Option<usize>,
) -> Result<Vec<PythonInterpreter>> {
    match bridge {
        BridgeModel::Bindings(binding_name) => {
            let mut interpreter = if !interpreter.is_empty() {
                PythonInterpreter::check_executables(&interpreter, &target, &bridge)
                    .context("The given list of python interpreters is invalid")?
            } else {
                PythonInterpreter::find_all(&target, &bridge, min_python_minor)
                    .context("Finding python interpreters failed")?
            };

            if interpreter.is_empty() {
                bail!("Couldn't find any python interpreters. Please specify at least one with -i");
            }

            if binding_name == "pyo3" && target.is_unix() && is_cross_compiling(target)? {
                if let Some(cross_lib_dir) = std::env::var_os("PYO3_CROSS_LIB_DIR") {
                    println!("⚠ Cross-compiling is poorly supported");
                    let host_python = &interpreter[0];
                    println!(
                        "🐍 Using host {} for cross-compiling preparation",
                        host_python
                    );
                    // pyo3
                    env::set_var("PYO3_PYTHON", &host_python.executable);
                    // rust-cpython, and legacy pyo3 versions
                    env::set_var("PYTHON_SYS_EXECUTABLE", &host_python.executable);

                    let sysconfig_path = find_sysconfigdata(cross_lib_dir.as_ref(), target)?;
                    let sysconfig_data =
                        parse_sysconfigdata(&host_python.executable, sysconfig_path)?;
                    let major = sysconfig_data
                        .get("version_major")
                        .context("version_major is not defined")?
                        .parse::<usize>()
                        .context("Could not parse value of version_major")?;
                    let minor = sysconfig_data
                        .get("version_minor")
                        .context("version_minor is not defined")?
                        .parse::<usize>()
                        .context("Could not parse value of version_minor")?;
                    let abiflags = sysconfig_data
                        .get("ABIFLAGS")
                        .map(ToString::to_string)
                        .unwrap_or_default();
                    let ext_suffix = sysconfig_data
                        .get("EXT_SUFFIX")
                        .context("syconfig didn't define an `EXT_SUFFIX` ಠ_ಠ")?;
                    let abi_tag = sysconfig_data
                        .get("SOABI")
                        .and_then(|abi| abi.split('-').nth(1).map(ToString::to_string));
                    let interpreter_kind = sysconfig_data
                        .get("SOABI")
                        .and_then(|tag| {
                            if tag.starts_with("pypy") {
                                Some(InterpreterKind::PyPy)
                            } else if tag.starts_with("cpython") {
                                Some(InterpreterKind::CPython)
                            } else {
                                None
                            }
                        })
                        .context("unsupported Python interpreter")?;
                    interpreter = vec![PythonInterpreter {
                        major,
                        minor,
                        abiflags,
                        target: target.clone(),
                        executable: PathBuf::new(),
                        ext_suffix: ext_suffix.to_string(),
                        interpreter_kind,
                        abi_tag,
                        libs_dir: PathBuf::from(cross_lib_dir),
                    }];
                }
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
            let interpreter = find_single_python_interpreter(bridge, interpreter, target, "cffi")?;
            println!("🐍 Using {} to generate the cffi bindings", interpreter);
            Ok(vec![interpreter])
        }
        BridgeModel::Bin => Ok(vec![]),
        BridgeModel::BindingsAbi3(major, minor) => {
            // Ideally, we wouldn't want to use any python interpreter without abi3 at all.
            // Unfortunately, on windows we need one to figure out base_prefix for a linker
            // argument.
            if target.is_windows() {
                if let Some(manual_base_prefix) = std::env::var_os("PYO3_CROSS_LIB_DIR") {
                    // PYO3_CROSS_LIB_DIR should point to the `libs` directory inside base_prefix
                    // when cross compiling, so we fake a python interpreter matching it
                    println!("⚠ Cross-compiling is poorly supported");
                    Ok(vec![PythonInterpreter {
                        major: *major as usize,
                        minor: *minor as usize,
                        abiflags: "".to_string(),
                        target: target.clone(),
                        executable: PathBuf::new(),
                        ext_suffix: ".pyd".to_string(),
                        interpreter_kind: InterpreterKind::CPython,
                        abi_tag: None,
                        libs_dir: PathBuf::from(manual_base_prefix),
                    }])
                } else {
                    let interpreter = find_single_python_interpreter(
                        bridge,
                        interpreter,
                        target,
                        "abi3 on windows",
                    )?;
                    println!("🐍 Using {} to generate to link bindings (With abi3, an interpreter is only required on windows)", interpreter);
                    Ok(vec![interpreter])
                }
            } else {
                println!("🐍 Not using a specific python interpreter (With abi3, an interpreter is only required on windows)");
                Ok(vec![])
            }
        }
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

/// We need to pass the global flags to cargo metadata
/// (https://github.com/PyO3/maturin/issues/211 and https://github.com/PyO3/maturin/issues/472),
/// but we can't pass all the extra args, as e.g. `--target` isn't supported, so this tries to
/// extract the arguments for cargo metadata
///
/// There are flags (without value) and options (with value). The options value be passed
/// in the same string as its name or in the next one. For this naive parsing logic, we
/// assume that the value is in the next argument if the argument string equals the name,
/// otherwise it's in the same argument and the next argument is unrelated.
fn extract_cargo_metadata_args(cargo_extra_args: &[String]) -> Result<Vec<String>> {
    // flags name and whether it has a value
    let known_prefixes = vec![
        ("--frozen", false),
        ("--locked", false),
        ("--offline", false),
        ("-Z", true),
        ("--features", true),
        ("--all-features", false),
        ("--no-default-features", false),
    ];
    let mut cargo_metadata_extra_args = vec![];
    let mut args_iter = cargo_extra_args.iter();
    // We do manual iteration so we can take and skip the value of an option that is in the next
    // argument
    while let Some(arg) = args_iter.next() {
        // Does it match any of the cargo metadata arguments?
        if let Some((prefix, has_arg)) = known_prefixes
            .iter()
            .find(|(prefix, _)| arg.starts_with(prefix))
        {
            cargo_metadata_extra_args.push(arg.to_string());
            // Do we also need to take the next argument?
            if arg == prefix && *has_arg {
                let value = args_iter.next().context(format!(
                    "Can't parse cargo-extra-args: {} is expected to have an argument",
                    prefix
                ))?;
                cargo_metadata_extra_args.push(value.to_owned());
            }
        }
    }
    Ok(cargo_metadata_extra_args)
}

#[cfg(test)]
mod test {
    use std::path::Path;

    use super::*;

    #[test]
    fn test_find_bridge_pyo3() {
        let pyo3_mixed = MetadataCommand::new()
            .manifest_path(&Path::new("test-crates/pyo3-mixed").join("Cargo.toml"))
            .exec()
            .unwrap();

        assert!(matches!(
            find_bridge(&pyo3_mixed, None),
            Ok(BridgeModel::Bindings(_))
        ));
        assert!(matches!(
            find_bridge(&pyo3_mixed, Some("pyo3")),
            Ok(BridgeModel::Bindings(_))
        ));

        assert!(find_bridge(&pyo3_mixed, Some("rust-cpython")).is_err());
    }

    #[test]
    fn test_find_bridge_pyo3_abi3() {
        let pyo3_pure = MetadataCommand::new()
            .manifest_path(&Path::new("test-crates/pyo3-pure").join("Cargo.toml"))
            .exec()
            .unwrap();

        assert!(matches!(
            find_bridge(&pyo3_pure, None),
            Ok(BridgeModel::BindingsAbi3(3, 6))
        ));
        assert!(matches!(
            find_bridge(&pyo3_pure, Some("pyo3")),
            Ok(BridgeModel::BindingsAbi3(3, 6))
        ));
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

        assert!(matches!(
            find_bridge(&pyo3_pure, None).unwrap(),
            BridgeModel::Bindings(_)
        ));
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
    fn test_old_extra_feature_args() {
        let cargo_extra_args = "--no-default-features --features a --target x86_64-unknown-linux-musl --features=c --lib";
        let cargo_extra_args = split_extra_args(&[cargo_extra_args.to_string()]).unwrap();
        let cargo_metadata_extra_args = extract_cargo_metadata_args(&cargo_extra_args).unwrap();
        assert_eq!(
            cargo_metadata_extra_args,
            vec!["--no-default-features", "--features", "a", "--features=c"]
        );
    }

    #[test]
    fn test_extract_cargo_metadata_args() {
        let args: Vec<_> = vec![
            "--locked",
            "--features=my-feature",
            "--unbeknownst",
            "--features",
            "other-feature",
            "--target",
            "x86_64-unknown-linux-musl",
            "-Zunstable-options",
        ]
        .iter()
        .map(ToString::to_string)
        .collect();

        let expected = vec![
            "--locked",
            "--features=my-feature",
            "--features",
            "other-feature",
            "-Zunstable-options",
        ];

        assert_eq!(extract_cargo_metadata_args(&args).unwrap(), expected);
    }

    #[test]
    fn test_get_min_python_minor() {
        // Nothing specified
        let cargo_toml = CargoToml::from_path("test-crates/pyo3-pure/Cargo.toml").unwrap();
        let metadata21 =
            Metadata21::from_cargo_toml(&cargo_toml, &"test-crates/pyo3-pure").unwrap();
        assert_eq!(get_min_python_minor(&metadata21), None);
    }
}
