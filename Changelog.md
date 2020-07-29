# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) (for the cli, not for the crate).

## 0.8.3 - 2020-07-29

### Added

 * `[tool.maturin]` now supports `sdist-include = ["path/**/*"]` to
include arbitrary files in source distributions ([#296](https://github.com/PyO3/maturin/pull/296)).
 * Add support for PyO3 `0.12`'s `PYO3_PYTHON` environment variable. [#331](https://github.com/PyO3/maturin/pull/331)
* Enable mixed layouts with Python source in different sub-directories. Either through `tools.maturin.py-src`
  in `pyproject.toml` or through the `--py-src` flag on `maturin`. 
  ([#335](https://github.com/PyO3/maturin/pull/335))

### Fixed

 * Fix incorrectly returning full path (not basename) from PEP 517 `build_sdist` hook

## 0.8.2 - 2020-06-29

### Added

 * Python 3.8 was added to PATH in the docker image by oconnor663 in [#302](https://github.com/PyO3/maturin/pull/302)

## 0.8.1 - 2020-04-30

### Added

 * cffi is installed if it's missing and python is running inside a virtualenv.

## 0.8.0 - 2020-04-03

### Added

 * There is now a binary wheel for aarch64
 * Warn if there are local dependencies

### Fixed

 * Omit author_email if `@` is not found in authors by evandrocoan in [#290](https://github.com/PyO3/maturin/pull/290)

## 0.7.9 - 2020-03-06

### Fixed

 * This release includes binary wheels for mac os

## 0.7.8 - 2020-03-06

### Added

 * Added support from arm, specifically arm7l, aarch64 by ijl in [#273](https://github.com/PyO3/maturin/pull/273)
 * Added support for manylinux2014 by ijl in [#273](https://github.com/PyO3/maturin/pull/273)

### Fixed

 * Remove python 2 from tags by ijl in [#254](https://github.com/PyO3/maturin/pull/254)
 * 32-bit wheels didn't work on linux. This has been fixed by dae in [#250](https://github.com/PyO3/maturin/pull/250)
 * The path of the RECORD file on windows used a backward slash instead of a forward slash

## 0.7.7 - 2019-11-12

### Added

 * The setup.py installer for bootstrapping maturin now checks for cargo instead of failing with a complex error message.
 * Upload errors now show the filesize

### Changed

* maturin's metadata now lists a requirement of python3.5 or later to install.

## [0.7.6] - 2019-09-28

### Changed

 * Only `--features`, `--no-default-features` and `--all-features` in `--cargo-extra-args` are passed to `cargo metadata` when determining the bindings, fixing problems in the previous release with arguments supported by `cargo build` but by `cargo metadata`.

## [0.7.5] - 2019-09-24

### Fixed

 * Fix clippy error to fix publishing from ci

## [0.7.4] - 2019-09-22

### Fixed

 * Fix tests

## [0.7.3] - 2019-09-22

### Fixed

 * Fix building when the bindings crate is behind a feature flag

## [0.7.3] - 2019-09-22

## Removed

 * The manylinux docker container doesn't contain musl anymore. If you're targeting musl, there's no need to use manylinux.

## [0.7.2] - 2019-09-05

### Added

 * Allow cross compilation with cffi and a python interpreter with the host target

### Fixed

 * Renamed a folder to maturin so PEP 517 backend works again.

## [0.7.1] - 2019-08-31

### Added

 * `maturin build --interpreter`/`maturin publish --interpreter` builds only a source distribution.

## [0.7.0] - 2019-08-30

With this release, the name of this project changes from _pyo3-pack_ to _maturin_.

### Added

 * Mixed rust/python layout
 * Added PEP 517 support
 * Added a `maturin sdist` command as workaround for [pypa/pip#6041](https://github.com/pypa/pip/issues/6041)
 * Support settings all applicable fields from the python core metadata specification in Cargo.toml
 * Support for FreeBSD by kxepal [#173](https://github.com/PyO3/maturin/pull/173)

## [0.6.1]

### Fixed

 * Downgraded to structopt 0.2.16 to avoid the yanked 0.2.17

## [0.6.0]

### Added

 * Basic pypy support by ijl [#105](https://github.com/PyO3/maturin/pull/105)

### Removed

 * Python 2 support
 * The custom progress bar was removed and cargo's output is shown instead

## [0.5.0]

### Added

 * Support for conda environments on windows by paddyhoran [#52](https://github.com/PyO3/maturin/pull/52)
 * maturin will generate a header for cffi crates using cbinding, which means you don't need a `build.rs` anymore. The option to provide your own header file using a `build.rs` still exists.
 * The [konstin2/maturin](https://cloud.docker.com/u/konstin2/repository/docker/konstin2/maturin) docker image makes it easy to build fully manylinux compliant wheels. See the readme for usage details.
 * Support for manylinux2010 by ijl [#70](https://github.com/PyO3/maturin/pull/70)
 * The `--manxlinux=[1|1-unchecked|2010|2010-unchecked|off]` option allows to build for manylinux1 and manylinux2010, both with audithweel (`1` or `2010`) and without (`1-unchecked` or `2010-unchecked`), but also for the native linux tag with `off`.

### Changed

 * The `--skip-auditwheel` flag has been deprecated in favor of `--manxlinux=[1|1-unchecked|2010|2010-unchecked|off]`.
 * Switched to rustls. This means the upload feature can be used from the docker container and builds of maturin itself are manylinux compliant when compiled with the musl target.

## [0.4.2] - 2018-12-15

Fixup release because the appveyor failed to release artifacts for windows for 0.4.1.

## [0.4.1] - 2018-12-15

### Added

 * You can now specify [trove classifiers](https://pypi.org/classifiers/) in your Cargo.toml with `package.metadata.maturin.classifier`. Implemented by ijl in [#48](https://github.com/PyO3/maturin/pull/48). Example:
 ```toml
  [package.metadata.maturin]
  classifier = ["Programming Language :: Python"]
  ```

## [0.4.0] - 2018-11-20

### Changed

 * publish defaults to release and strip, unless `--debug` or `--no-strip` are given.

### Added

 * New ci script based on hyperfine which also builds debian packages.

## [0.3.10] - 2018-11-16

### Fixed

 * Fix rust-cpython detection and compilation

## [0.3.9]

### Changed

 * Update reqwest to 0.9.4 which has [seanmonstar/reqwest#374](https://github.com/seanmonstar/reqwest/issues/374) fixed

## [0.3.8]

### Fixed

 * Pin reqwest to 0.9.2 to work around [seanmonstar/reqwest#374](https://github.com/seanmonstar/reqwest/issues/374)

## [0.3.7]

### Fixed

 * Added cargo lock to project [#9](https://github.com/PyO3/maturin/issues/9)

## [0.3.6]

With deflate and the strip options, the wheels get about 25x smaller:

wheel | baseline | deflate | strip + deflate
-|-|-|-
get_fourtytwo-2.0.1-cp36-cp36m-manylinux1_x86_64.whl | 2,8M | 771K | 102K
hello_world-0.1.0-py2.py3-none-manylinux1_x86_64.whl | 3,9M | 1,1M | 180K
points-0.1.0-py2.py3-none-manylinux1_x86_64.whl | 2,8M | 752K | 85K

### Added

 * `--strip` by ijl [#7](https://github.com/PyO3/maturin/pull/7)

### Changed

 * Renamed `--bindings-crate` to `--bindings`
 * Use deflate compression for zips by ijl [#6](https://github.com/PyO3/maturin/pull/6)

### Fixed

 * `--target` is now actually used for the wheel compatibility tag

## [0.3.5] - 2018-09-20

### Changed

 * Upgraded to reqwest 0.9

### Fixed

 * "Broken Pipe" with musl builds (through the reqwest upgrade)

## [0.3.4] - 2018-09-18

### Added

 * A `--target` option which behaves like cargo option of the same name

### Changed

 * Musl and auditwheel compliance: Using the new `musl` feature combined with the musl target, you can build completely static binaries. The `password-storage`, which enables keyring integration, is now disabled by default. The Pypi packages are now statically linked with musl so that they are audtiwheel compliant.
 * Replaced `--debug` with `--release`. All builds are now debug by default

## [0.3.3] - 2018-09-17

### Added

 * Builds for i686 linux and mac
 * Builds for maturin as wheel

## Fixed

 * Usage with stable
 * Wrong tags in WHEEL file on non-linux platforms
 * Uploading on windows

## [0.3.1] - 2017-09-14

### Fixed

 * Windows compilation

## [0.3.0] - 2017-09-14

### Added

 * Packaging binaries
 * [Published on pypi](https://pypi.org/project/maturin/). You can now `pip install maturin`
 * A Dockerfile based on manylinux1

### Fixed

 * Travis ci setup builds all types of wheels for linux and mac
 * `--no-default-features --features auditwheel` creates a manylinux compliant binary for maturin

### Changed

 * Replaced elfkit with goblin

## [0.2.0] - 2018-09-03

### Added

 * Cffi support
 * A `develop` subcommand
 * A tox example

### Changed

 * Show a progress bar for cargo's compile progress

## 0.1.0 - 2018-08-22

 * Initial Release

[Unreleased]: https://github.com/pyo3/maturin/compare/v0.7.6...HEAD
[0.7.6]: https://github.com/pyo3/maturin/compare/v0.7.5...0.7.6
[0.7.5]: https://github.com/pyo3/maturin/compare/v0.7.4...0.7.5
[0.7.4]: https://github.com/pyo3/maturin/compare/v0.7.3...0.7.4
[0.7.3]: https://github.com/pyo3/maturin/compare/v0.7.2...0.7.3
[0.7.2]: https://github.com/pyo3/maturin/compare/v0.7.1...0.7.2
[0.7.1]: https://github.com/pyo3/maturin/compare/v0.7.0...0.7.1
[0.7.0]: https://github.com/pyo3/maturin/compare/v0.6.1...0.7.0
[0.6.1]: https://github.com/pyo3/maturin/compare/v0.6.0...0.6.1
[0.6.0]: https://github.com/pyo3/maturin/compare/v0.5.0...0.6.0
[0.5.0]: https://github.com/pyo3/maturin/compare/v0.4.2...v0.5.0
[0.4.2]: https://github.com/pyo3/maturin/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/pyo3/maturin/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/pyo3/maturin/compare/v0.3.10...v0.4.0
[0.3.10]: https://github.com/pyo3/maturin/compare/v0.3.9...v0.3.10
[0.3.9]: https://github.com/pyo3/maturin/compare/v0.3.8...v0.3.9
[0.3.8]: https://github.com/pyo3/maturin/compare/v0.3.7...v0.3.8
[0.3.7]: https://github.com/pyo3/maturin/compare/v0.3.6...v0.3.7
[0.3.6]: https://github.com/pyo3/maturin/compare/v0.3.5...v0.3.5
[0.3.5]: https://github.com/pyo3/maturin/compare/v0.3.4...v0.3.5
[0.3.4]: https://github.com/pyo3/maturin/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/pyo3/maturin/compare/v0.3.1...v0.3.3
[0.3.1]: https://github.com/pyo3/maturin/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/pyo3/maturin/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/pyo3/maturin/compare/v0.1.0...v0.2.0
