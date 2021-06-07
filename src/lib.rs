//! Builds wheels from a crate that exposes python bindings through pyo3
//!
//! The high-level api is [BuildOptions], which can be converted into the [BuildContext], which
//! then uses [compile()] and builds the appropriate wheels.
//!
//! # Cargo features
//!
//! Default features: auditwheel, log, upload, rustls
//!
//! - auditwheel: Reimplements the more important part of the auditwheel
//! package in rust. A wheel is checked by default, unless deactivated by cli arguments
//!
//! - log: Configures pretty-env-logger, even though maturin doesn't use logging itself.
//!
//! - upload: Uses reqwest to add the upload command.
//!
//! - rustls: Makes reqwest use the rustls stack so that we can build maturin in a CentOS 6
//! docker container and which maturin itself manylinux compliant.
//!
//! - human-panic: Adds https://github.com/rust-clique/human-panic
//!
//! - password-storage (off by default): Uses the keyring package to store the password. keyring
//! pulls in a lot of shared libraries and outdated dependencies, so this is off by default, except
//! for the build on the github releases page.
//! (https://github.com/hwchen/secret-service-rs/issues/9)

#![deny(missing_docs)]

pub use crate::auditwheel::{auditwheel_rs, AuditWheelError};
pub use crate::build_context::{BridgeModel, BuildContext, BuiltWheelMetadata};
pub use crate::build_options::BuildOptions;
pub use crate::cargo_toml::CargoToml;
pub use crate::compile::compile;
pub use crate::develop::develop;
pub use crate::metadata::{Metadata22, WheelMetadata};
pub use crate::module_writer::{
    write_dist_info, ModuleWriter, PathWriter, SDistWriter, WheelWriter,
};
pub use crate::pyproject_toml::PyProjectToml;
pub use crate::python_interpreter::PythonInterpreter;
pub use crate::read_distribution::{
    get_metadata_for_distribution, get_supported_version_for_distribution,
};
pub use crate::target::Target;
pub use auditwheel::PlatformTag;
pub use source_distribution::source_distribution;
#[cfg(feature = "upload")]
pub use {
    crate::registry::Registry,
    crate::upload::{upload, UploadError},
};

mod auditwheel;
mod build_context;
mod build_options;
mod cargo_toml;
mod compile;
mod cross_compile;
mod develop;
mod metadata;
mod module_writer;
mod pyproject_toml;
mod python_interpreter;
mod read_distribution;
#[cfg(feature = "upload")]
mod registry;
mod source_distribution;
mod target;
#[cfg(feature = "upload")]
mod upload;
