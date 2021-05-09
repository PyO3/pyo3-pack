use crate::auditwheel::Manylinux;
use crate::{BridgeModel, Target};
use anyhow::{bail, format_err, Context, Result};
use regex::Regex;
use serde::Deserialize;
use std::collections::HashSet;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str;

/// This snippets will give us information about the python interpreter's
/// version and abi as json through stdout
const GET_INTERPRETER_METADATA: &str = include_str!("get_interpreter_metadata.py");
const MINIMUM_PYTHON_MINOR: usize = 6;
/// Be liberal here to include preview versions
const MAXIMUM_PYTHON_MINOR: usize = 12;

/// Identifies conditions where we do not want to build wheels
fn windows_interpreter_no_build(
    major: usize,
    minor: usize,
    target_width: usize,
    pointer_width: usize,
    min_python_minor: usize,
) -> bool {
    // Python 2 support has been dropped
    if major == 2 {
        return true;
    }

    // Ignore python 3.0 - 3.5
    if major == 3 && minor < min_python_minor {
        return true;
    }

    // There can be 32-bit installations on a 64-bit machine, but we can't link
    // those for 64-bit targets
    if pointer_width != target_width {
        println!(
            "👽 {}.{} is installed as {}-bit, while the target is {}-bit. Skipping.",
            major, minor, pointer_width, target_width
        );
        return true;
    }
    false
}

/// On windows regular Python installs are supported along with environments
/// being managed by `conda`.
///
/// We can't use the linux trick with trying different binary names since on
/// windows the binary is always called "python.exe".  However, whether dealing
/// with regular Python installs or `conda` environments there are tools we can
/// use to query the information regarding installed interpreters.
///
/// Regular Python installs downloaded from Python.org will include the python
/// launcher by default.  We can use the launcher to find the information we need
/// for each installed interpreter using `py -0` which produces something like
/// the following output (the path can by determined using `sys.executable`):
///
/// ```bash
/// Installed Pythons found by py Launcher for Windows
/// -3.7-64 *
/// -3.6-32
/// ```
///
/// When using `conda` we can use the `conda info -e` command to retrieve information
/// regarding the installed interpreters being managed by `conda`.  This is an example
/// of the output expected:
///
/// ```bash
/// # conda environments:
/// #
/// base                     C:\Users\<user-name>\Anaconda3
/// foo1                  *  C:\Users\<user-name>\Anaconda3\envs\foo1
/// foo2                  *  C:\Users\<user-name>\Anaconda3\envs\foo2
/// ```
///
/// The information required can either by obtained by parsing this output directly or
/// by invoking the interpreters to obtain the information.
///
/// As well as the version numbers, etc. of the interpreters we also have to find the
/// pointer width to make sure that the pointer width (32-bit or 64-bit) matches across
/// platforms.
fn find_all_windows(target: &Target, min_python_minor: usize) -> Result<Vec<String>> {
    let code = "import sys; print(sys.executable or '')";
    let mut interpreter = vec![];
    let mut versions_found = HashSet::new();

    // If Python is installed from Python.org it should include the "python launcher"
    // which is used to find the installed interpreters
    let execution = Command::new("py").arg("-0").output();
    if let Ok(output) = execution {
        let expr = Regex::new(r" -(\d).(\d)-(\d+)(?: .*)?").unwrap();
        let lines = str::from_utf8(&output.stdout).unwrap().lines();
        for line in lines {
            if let Some(capture) = expr.captures(line) {
                let context = "Expected a digit";

                let major = capture
                    .get(1)
                    .unwrap()
                    .as_str()
                    .parse::<usize>()
                    .context(context)?;
                let minor = capture
                    .get(2)
                    .unwrap()
                    .as_str()
                    .parse::<usize>()
                    .context(context)?;
                if !versions_found.contains(&(major, minor)) {
                    let pointer_width = capture
                        .get(3)
                        .unwrap()
                        .as_str()
                        .parse::<usize>()
                        .context(context)?;

                    if windows_interpreter_no_build(
                        major,
                        minor,
                        target.pointer_width(),
                        pointer_width,
                        min_python_minor,
                    ) {
                        continue;
                    }

                    let version = format!("-{}.{}-{}", major, minor, pointer_width);

                    let output = Command::new("py")
                        .args(&[&version, "-c", code])
                        .output()
                        .unwrap();
                    let path = str::from_utf8(&output.stdout).unwrap().trim();
                    if !output.status.success() || path.trim().is_empty() {
                        bail!("Couldn't determine the path to python for `py {}`", version);
                    }
                    interpreter.push(path.to_string());
                    versions_found.insert((major, minor));
                }
            }
        }
    }

    // Conda environments are also supported on windows
    let conda_info = Command::new("conda").arg("info").arg("-e").output();
    if let Ok(output) = conda_info {
        let lines = str::from_utf8(&output.stdout).unwrap().lines();
        // The regex has three parts: The first matches the name and skips
        // comments, the second skips the part in between and the third
        // extracts the path
        let re = Regex::new(r"^([^#].*?)[\s*]+([\w\\:.-]+)\s*$").unwrap();
        let mut paths = vec![];
        for i in lines {
            if let Some(capture) = re.captures(&i) {
                if &capture[1] == "base" {
                    continue;
                }
                paths.push(String::from(&capture[2]));
            }
        }

        for path in paths {
            let executable = Path::new(&path).join("python");
            let python_info = Command::new(&executable)
                .arg("-c")
                .arg("import sys; print(sys.version)")
                .output();

            let python_info = match python_info {
                Ok(python_info) => python_info,
                Err(err) => {
                    if err.kind() == io::ErrorKind::NotFound {
                        // This conda env doesn't have python installed
                        continue;
                    } else {
                        bail!(
                            "Error getting Python version info from conda env at {}",
                            path
                        );
                    }
                }
            };

            let version_info = str::from_utf8(&python_info.stdout).unwrap();
            let expr = Regex::new(r"(\d).(\d).(\d+)").unwrap();
            if let Some(capture) = expr.captures(version_info) {
                let major = capture.get(1).unwrap().as_str().parse::<usize>().unwrap();
                let minor = capture.get(2).unwrap().as_str().parse::<usize>().unwrap();
                if !versions_found.contains(&(major, minor)) {
                    let pointer_width = if version_info.contains("64 bit (AMD64)") {
                        64_usize
                    } else {
                        32_usize
                    };

                    if windows_interpreter_no_build(
                        major,
                        minor,
                        target.pointer_width(),
                        pointer_width,
                        min_python_minor,
                    ) {
                        continue;
                    }

                    interpreter.push(String::from(executable.to_str().unwrap()));
                    versions_found.insert((major, minor));
                }
            }
        }
    }
    if interpreter.is_empty() {
        bail!(
            "Could not find any interpreters, are you sure you have python installed on your PATH?"
        );
    };
    Ok(interpreter)
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum InterpreterKind {
    CPython,
    PyPy,
}

impl fmt::Display for InterpreterKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            InterpreterKind::CPython => write!(f, "CPython"),
            InterpreterKind::PyPy => write!(f, "PyPy"),
        }
    }
}

/// The output format of [GET_INTERPRETER_METADATA]
#[derive(Deserialize)]
struct IntepreterMetadataMessage {
    major: usize,
    minor: usize,
    abiflags: Option<String>,
    interpreter: String,
    ext_suffix: Option<String>,
    platform: String,
    abi_tag: Option<String>,
    base_prefix: String,
}

/// The location and version of an interpreter
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct PythonInterpreter {
    /// Python's major version
    pub major: usize,
    /// Python's minor version
    pub minor: usize,
    /// For linux and mac, this contains the value of the abiflags, e.g. "m"
    /// for python3.7m or "dm" for python3.6dm. Since python3.8, the value is
    /// empty. On windows, the value was always "".
    ///
    /// See PEP 261 and PEP 393 for details
    pub abiflags: String,
    /// Currently just the value of [Target::os()], i.e. "windows", "linux",
    /// "macos" or "freebsd"
    pub target: Target,
    /// Path to the python interpreter, e.g. /usr/bin/python3.6
    ///
    /// Just the name of the binary in PATH does also work, e.g. `python3.5`
    pub executable: PathBuf,
    /// Suffix to use for extension modules as given by sysconfig.
    pub ext_suffix: String,
    /// cpython or pypy
    pub interpreter_kind: InterpreterKind,
    /// Part of sysconfig's SOABI specifying {major}{minor}{abiflags}
    ///
    /// Note that this always `None` on windows
    pub abi_tag: Option<String>,
    /// We need this value for windows abi3 linking
    pub libs_dir: PathBuf,
}

/// Returns the abiflags that are assembled through the message, with some
/// additional sanity checks.
///
/// The rules are as follows:
///  - python 3 + Unix: Use ABIFLAGS
///  - python 3 + Windows: No ABIFLAGS, return an empty string
fn fun_with_abiflags(
    message: &IntepreterMetadataMessage,
    target: &Target,
    bridge: &BridgeModel,
) -> Result<String> {
    if bridge != &BridgeModel::Cffi && target.get_python_os() != message.platform {
        bail!(
            "sys.platform in python, {}, and the rust target, {:?}, don't match ಠ_ಠ",
            message.platform,
            target,
        )
    }

    if message.major != 3 || message.minor < 5 {
        bail!(
            "Only python >= 3.5 is supported, while you're using python {}.{}",
            message.major,
            message.minor
        );
    }

    if message.interpreter == "pypy" {
        // pypy does not specify abi flags
        Ok("".to_string())
    } else if message.platform == "windows" {
        if message.abiflags.is_some() {
            bail!("A python 3 interpreter on windows does not define abiflags in its sysconfig ಠ_ಠ")
        } else {
            Ok("".to_string())
        }
    } else if let Some(ref abiflags) = message.abiflags {
        if message.minor >= 8 {
            // for 3.8, "builds with and without pymalloc are ABI compatible" and the flag dropped
            Ok(abiflags.to_string())
        } else if (abiflags != "m") && (abiflags != "dm") {
            bail!("A python 3 interpreter on linux or mac os must have 'm' or 'dm' as abiflags ಠ_ಠ")
        } else {
            Ok(abiflags.to_string())
        }
    } else {
        bail!("A python 3 interpreter on linux or mac os must define abiflags in its sysconfig ಠ_ಠ")
    }
}

impl PythonInterpreter {
    /// Returns the supported python environment in the PEP 425 format used for the wheel filename:
    /// {python tag}-{abi tag}-{platform tag}
    ///
    /// Don't ask me why or how, this is just what setuptools uses so I'm also going to use
    ///
    /// If abi3 is true, cpython wheels use the generic abi3 with the given version as minimum
    pub fn get_tag(&self, manylinux: Manylinux, universal2: bool) -> String {
        match self.interpreter_kind {
            InterpreterKind::CPython => {
                let platform = self.target.get_platform_tag(manylinux, universal2);
                if self.target.is_unix() {
                    format!(
                        "cp{major}{minor}-cp{major}{minor}{abiflags}-{platform}",
                        major = self.major,
                        minor = self.minor,
                        abiflags = self.abiflags,
                        platform = platform
                    )
                } else {
                    // On windows the abiflags are missing, but this seems to work
                    format!(
                        "cp{major}{minor}-none-{platform}",
                        major = self.major,
                        minor = self.minor,
                        platform = platform
                    )
                }
            }
            InterpreterKind::PyPy => {
                let platform = self.target.get_platform_tag(manylinux, universal2);
                // pypy uses its version as part of the ABI, e.g.
                // pypy 3.7 7.3 => numpy-1.20.1-pp37-pypy37_pp73-manylinux2010_x86_64.whl
                format!(
                    "pp{major}{minor}-pypy{major}{minor}_{abi_tag}-{platform}",
                    major = self.major,
                    minor = self.minor,
                    // TODO: Proper tag handling for pypy
                    abi_tag = self
                        .abi_tag
                        .clone()
                        .expect("PyPy's syconfig didn't define an `SOABI` ಠ_ಠ"),
                    platform = platform,
                )
            }
        }
    }

    /// Adds the ext_suffix we read from python or know (.pyd/.abi3.so) and adds it to the base name
    ///
    /// For CPython, generate extensions as follows:
    ///
    /// For python 3, there is PEP 3149, but
    /// that is only valid for 3.2 - 3.4. Since only 3.6+ is supported, the
    /// templates are adapted from the (also
    /// incorrect) release notes of CPython 3.5:
    /// https://docs.python.org/3/whatsnew/3.5.html#build-and-c-api-changes
    ///
    /// Examples for 64-bit on CPython 3.6m:
    /// Linux:   foobar.cpython-36m-x86_64-linux-gnu.so
    /// Windows: foobar.cp36-win_amd64.pyd
    /// Mac:     foobar.cpython-36m-darwin.so
    /// FreeBSD: foobar.cpython-36m.so
    ///
    /// For pypy3, we read importlib.machinery.EXTENSION_SUFFIXES[0].
    pub fn get_library_name(&self, base: &str) -> String {
        format!(
            "{base}{ext_suffix}",
            base = base,
            ext_suffix = self.ext_suffix
        )
    }

    /// Checks whether the given command is a python interpreter and returns a
    /// [PythonInterpreter] if that is the case
    pub fn check_executable(
        executable: impl AsRef<Path>,
        target: &Target,
        bridge: &BridgeModel,
    ) -> Result<Option<PythonInterpreter>> {
        let output = match which::which(&executable.as_ref()) {
            Ok(path) => Command::new(path)
                .args(&["-c", GET_INTERPRETER_METADATA])
                .stderr(Stdio::inherit())
                .output(),
            Err(_err) => {
                return Ok(None);
            }
        };

        let err_msg = format!(
            "Trying to get metadata from the python interpreter '{}' failed",
            executable.as_ref().display()
        );
        let output = match output {
            Ok(output) => {
                if output.status.success() {
                    output
                } else {
                    bail!(err_msg);
                }
            }
            Err(err) => {
                if err.kind() == io::ErrorKind::NotFound {
                    return Ok(None);
                } else {
                    return Err(err).context(err_msg);
                }
            }
        };
        let message: IntepreterMetadataMessage = serde_json::from_slice(&output.stdout)
            .context(err_msg)
            .context(String::from_utf8_lossy(&output.stdout).trim().to_string())?;

        if (message.major == 2 && message.minor != 7) || (message.major == 3 && message.minor < 5) {
            return Ok(None);
        }

        let interpreter;
        match message.interpreter.as_str() {
            "cpython" => interpreter = InterpreterKind::CPython,
            "pypy" => interpreter = InterpreterKind::PyPy,
            _ => {
                bail!("Invalid interpreter");
            }
        };

        let abiflags = fun_with_abiflags(&message, &target, &bridge).context(format_err!(
            "Failed to get information from the python interpreter at {}",
            executable.as_ref().display()
        ))?;

        Ok(Some(PythonInterpreter {
            major: message.major,
            minor: message.minor,
            abiflags,
            target: target.clone(),
            executable: executable.as_ref().to_path_buf(),
            ext_suffix: message
                .ext_suffix
                .context("syconfig didn't define an `EXT_SUFFIX` ಠ_ಠ")?,
            interpreter_kind: interpreter,
            abi_tag: message.abi_tag,
            libs_dir: PathBuf::from(message.base_prefix).join("libs"),
        }))
    }

    /// Tries to find all installed python versions using the heuristic for the
    /// given platform
    pub fn find_all(
        target: &Target,
        bridge: &BridgeModel,
        min_python_minor: Option<usize>,
    ) -> Result<Vec<PythonInterpreter>> {
        let min_python_minor = min_python_minor.unwrap_or(MINIMUM_PYTHON_MINOR);
        let executables = if target.is_windows() {
            find_all_windows(&target, min_python_minor)?
        } else {
            (min_python_minor..MAXIMUM_PYTHON_MINOR)
                .map(|minor| format!("python3.{}", minor))
                .collect()
        };
        let mut available_versions = Vec::new();
        for executable in executables {
            if let Some(version) =
                PythonInterpreter::check_executable(&executable, &target, &bridge)?
            {
                available_versions.push(version);
            }
        }

        Ok(available_versions)
    }

    /// Checks that given list of executables are all valid python intepreters,
    /// determines the abiflags and versions of those interpreters and
    /// returns them as [PythonInterpreter]
    pub fn check_executables(
        executables: &[PathBuf],
        target: &Target,
        bridge: &BridgeModel,
    ) -> Result<Vec<PythonInterpreter>> {
        let mut available_versions = Vec::new();
        for executable in executables {
            if let Some(version) =
                PythonInterpreter::check_executable(executable, &target, &bridge).context(
                    format!("{} is not a valid python interpreter", executable.display()),
                )?
            {
                available_versions.push(version);
            } else {
                bail!(
                    "Python interpreter `{}` doesn't exist",
                    executable.display()
                );
            }
        }

        Ok(available_versions)
    }
}

impl fmt::Display for PythonInterpreter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}.{}{} at {}",
            self.interpreter_kind,
            self.major,
            self.minor,
            self.abiflags,
            self.executable.display()
        )
    }
}
