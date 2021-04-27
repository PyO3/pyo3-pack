//! To speed up the tests, they are tests all collected in a single module

use common::{develop, errors, handle_result, integration, other};

mod common;

#[test]
fn develop_pyo3_pure() {
    handle_result(develop::test_develop("test-crates/pyo3-pure", None));
}

#[test]
fn develop_pyo3_mixed() {
    handle_result(develop::test_develop("test-crates/pyo3-mixed", None));
}

#[test]
fn develop_pyo3_src_layout() {
    handle_result(develop::test_develop("test-crates/pyo3-src-layout", None));
}

#[test]
fn develop_cffi_pure() {
    handle_result(develop::test_develop("test-crates/cffi-pure", None));
}

#[test]
fn develop_cffi_mixed() {
    handle_result(develop::test_develop("test-crates/cffi-mixed", None));
}

#[test]
fn develop_hello_world() {
    handle_result(develop::test_develop("test-crates/hello-world", None));
}

#[test]
fn integration_pyo3_pure() {
    handle_result(integration::test_integration("test-crates/pyo3-pure", None));
}

#[test]
fn integration_pyo3_mixed() {
    handle_result(integration::test_integration(
        "test-crates/pyo3-mixed",
        None,
    ));
}

#[test]
fn integration_pyo3_src_layout() {
    handle_result(integration::test_integration(
        "test-crates/pyo3-src-layout",
        None,
    ));
}

#[cfg(target_os = "windows")]
#[test]
#[ignore]
fn integration_pyo3_pure_conda() {
    handle_result(integration::test_integration_conda(
        "text-crates/pyo3-pure",
        None,
    ));
}

#[test]
fn integration_cffi_pure() {
    handle_result(integration::test_integration("test-crates/cffi-pure", None));
}

#[test]
fn integration_cffi_mixed() {
    handle_result(integration::test_integration(
        "test-crates/cffi-mixed",
        None,
    ));
}

#[test]
fn integration_hello_world() {
    handle_result(integration::test_integration(
        "test-crates/hello-world",
        None,
    ));
}

#[test]
fn abi3_without_version() {
    handle_result(errors::abi3_without_version())
}

#[test]
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn pyo3_no_extension_module() {
    handle_result(errors::pyo3_no_extension_module())
}

#[test]
fn locked_doesnt_build_without_cargo_lock() {
    handle_result(errors::locked_doesnt_build_without_cargo_lock())
}

#[test]
#[cfg(target_os = "linux")]
fn musl() {
    let ran = handle_result(other::test_musl());
    if !ran {
        eprintln!("⚠  Warning: rustup and/or musl target not installed, test didn't run");
    }
}

#[test]
fn workspace_cargo_lock() {
    handle_result(other::test_workspace_cargo_lock())
}
