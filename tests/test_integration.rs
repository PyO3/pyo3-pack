use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str;

use anyhow::{bail, Context, Result};
use structopt::StructOpt;

use maturin::{BuildOptions, Target};

use crate::common::{adjust_canonicalization, check_installed, handle_result, maybe_mock_cargo};

mod common;

#[test]
fn test_integration_pyo3_pure() {
    handle_result(test_integration("test-crates/pyo3-pure", None, None));
}

#[test]
fn test_integration_pyo3_mixed() {
    handle_result(test_integration(
        "test-crates/pyo3-mixed",
        None,
        Some("python".into()),
    ));
}

#[cfg(target_os = "windows")]
#[test]
#[ignore]
fn test_integration_pyo3_pure_conda() {
    handle_result(test_integration_conda("text-crates/pyo3-pure", None, None));
}

#[test]
fn test_integration_cffi_pure() {
    handle_result(test_integration("test-crates/cffi-pure", None, None));
}

#[test]
fn test_integration_cffi_mixed() {
    handle_result(test_integration("test-crates/cffi-mixed", None, None));
}

#[test]
fn test_integration_hello_world() {
    handle_result(test_integration("test-crates/hello-world", None, None));
}

/// For each installed python version, this builds a wheel, creates a virtualenv if it
/// doesn't exist, installs the package and runs check_installed.py
fn test_integration(
    package: impl AsRef<Path>,
    bindings: Option<String>,
    py_src: Option<String>,
) -> Result<()> {
    maybe_mock_cargo();

    let target = Target::from_target_triple(None)?;

    let package_string = package.as_ref().join("Cargo.toml").display().to_string();

    // The first argument is ignored by clap
    let mut cli = vec![
        "build",
        "--manifest-path",
        &package_string,
        "--cargo-extra-args='--quiet'",
    ];

    if let Some(ref bindings) = bindings {
        cli.push("--bindings");
        cli.push(bindings);
    }

    if let Some(py_src) = py_src.as_ref() {
        cli.push("--py-src");
        cli.push(py_src);
    }

    let options = BuildOptions::from_iter_safe(cli)?;

    let wheels = options
        .into_build_context(false, cfg!(feature = "faster-tests"))?
        .build_wheels()?;

    let test_name = package
        .as_ref()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    for (filename, supported_version, python_interpreter) in wheels {
        let venv_name = if supported_version == "py3" {
            format!("{}-cffi", test_name)
        } else {
            format!(
                "{}-{}.{}",
                test_name,
                supported_version.chars().nth(2usize).unwrap(),
                supported_version.chars().nth(3usize).unwrap()
            )
        };
        let venv_dir = PathBuf::from("test-crates")
            .canonicalize()?
            .join("venvs")
            .join(venv_name);

        if !venv_dir.is_dir() {
            let output = if let Some(ref python_interpreter) = python_interpreter {
                Command::new("virtualenv")
                    .arg("-p")
                    .arg(python_interpreter.executable.clone())
                    .arg(&venv_dir)
                    .output()?
            } else {
                Command::new("virtualenv").arg(&venv_dir).output()?
            };
            if !output.status.success() {
                bail!(
                    "Failed to create a virtualenv at {}: {}\n--- Stdout:\n{}\n--- Stderr:\n{}",
                    venv_dir.display(),
                    output.status,
                    str::from_utf8(&output.stdout)?,
                    str::from_utf8(&output.stderr)?,
                );
            }
        }

        let python = target.get_venv_python(&venv_dir);

        let output = Command::new(&python)
            .args(&[
                "-m",
                "pip",
                "install",
                "--force-reinstall",
                &adjust_canonicalization(filename),
            ])
            .stderr(Stdio::inherit())
            .output()
            .context(format!("pip install failed with {:?}", python))?;
        if !output.status.success() {
            panic!();
        }

        check_installed(&package.as_ref(), &python)?;
    }

    Ok(())
}

/// Creates conda environments
#[cfg(target_os = "windows")]
fn create_conda_env(name: &str, major: usize, minor: usize) {
    Command::new("conda")
        .arg("create")
        .arg("-n")
        .arg(name)
        .arg(format!("python={}.{}", major, minor))
        .arg("-q")
        .arg("-y")
        .output()
        .expect("Conda not available.");
}

#[cfg(target_os = "windows")]
fn test_integration_conda(
    package: impl AsRef<Path>,
    bindings: Option<String>,
    py_src: Option<String>,
) -> Result<()> {
    use std::env;

    let package_string = package.as_ref().join("Cargo.toml").display().to_string();

    // Since the python launcher has precedence over conda, we need to deactivate it.
    // We do so by shadowing it with our own hello world binary.
    let original_path = env::var_os("PATH").expect("PATH is not defined");
    let py_dir = env::current_dir()?
        .join("test-data")
        .to_str()
        .unwrap()
        .to_string();
    let mocked_path = py_dir + ";" + original_path.to_str().unwrap();
    env::set_var("PATH", mocked_path);

    // Create environments to build against, prepended with "A" to ensure that integration
    // tests are executed with these environments
    create_conda_env("A-pyo3-build-env-35", 3, 5);
    create_conda_env("A-pyo3-build-env-36", 3, 6);
    create_conda_env("A-pyo3-build-env-37", 3, 7);

    // The first argument is ignored by clap
    let mut cli = vec![
        "build",
        "--manifest-path",
        &package_string,
        "--cargo-extra-args='--quiet'",
    ];

    if let Some(ref bindings) = bindings {
        cli.push("--bindings");
        cli.push(bindings);
    }

    if let Some(py_src) = py_src.as_ref() {
        cli.push("--py-src");
        cli.push(py_src);
    }

    let options = BuildOptions::from_iter_safe(cli)?;

    let wheels = options
        .into_build_context(false, cfg!(feature = "faster-tests"))?
        .build_wheels()?;

    let mut conda_wheels: Vec<(PathBuf, PathBuf)> = vec![];
    for (filename, _, python_interpreter) in wheels {
        if let Some(pi) = python_interpreter {
            let executable = pi.executable;
            if executable.to_str().unwrap().contains("pyo3-build-env-") {
                conda_wheels.push((filename, executable))
            }
        }
    }

    assert_eq!(
        3,
        conda_wheels.len(),
        "Error creating or detecting conda environments."
    );
    for (wheel_file, executable) in conda_wheels {
        let output = Command::new(&executable)
            .args(&[
                "-m",
                "pip",
                "install",
                "--force-reinstall",
                &adjust_canonicalization(wheel_file),
            ])
            .stderr(Stdio::inherit())
            .output()?;
        if !output.status.success() {
            panic!();
        }
        check_installed(&package.as_ref(), &executable)?;
    }

    Ok(())
}
