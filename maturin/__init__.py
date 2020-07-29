#!/usr/bin/env python3
"""
maturin's implementation of the PEP 517 interface. Calls maturin through subprocess

Currently, the "return value" of the rust implementation is the last line of stdout

On windows, apparently pip's subprocess handling sets stdout to some windows encoding (e.g. cp1252 on my machine),
even though the terminal supports utf8. Writing directly to the binary stdout buffer avoids ecoding errors due to
maturin's emojis.
"""

import os
import shutil
import subprocess
import sys
from subprocess import SubprocessError
from typing import List, Dict

import toml

# these are only used when creating the sdist, not when building it
create_only_options = [
    "sdist-include",
]

available_options = [
    "bindings",
    "cargo-extra-args",
    "manylinux",
    "rustc-extra-args",
    "skip-auditwheel",
    "strip",
    "py-src"
]


def get_config() -> Dict[str, str]:
    with open("pyproject.toml") as fp:
        pyproject_toml = toml.load(fp)
    return pyproject_toml.get("tool", {}).get("maturin", {})


def get_config_options() -> List[str]:
    config = get_config()
    options = []
    for key, value in config.items():
        if key in create_only_options:
            continue
        if key not in available_options:
            # attempt to install even if keys from newer or older versions are present
            print("WARNING: {} is not a recognized option for maturin".format(key))
        options.append("--{}={}".format(key, value))
    return options


# noinspection PyUnusedLocal
def build_wheel(wheel_directory, config_settings=None, metadata_directory=None):
    # The PEP 517 build environment guarantees that `python` is the correct python
    command = ["maturin", "pep517", "build-wheel", "-i", "python"]
    command.extend(get_config_options())

    print("Running `{}`".format(" ".join(command)))
    try:
        output = subprocess.check_output(command)
    except subprocess.CalledProcessError as e:
        print("Error: {}".format(e))
        sys.exit(1)
    sys.stdout.buffer.write(output)
    sys.stdout.flush()
    output = output.decode(errors="replace")
    filename = output.strip().splitlines()[-1]
    shutil.copy2("target/wheels/" + filename, os.path.join(wheel_directory, filename))
    return filename


# noinspection PyUnusedLocal
def build_sdist(sdist_directory, config_settings=None):
    command = ["maturin", "pep517", "write-sdist", "--sdist-directory", sdist_directory]

    print("Running `{}`".format(" ".join(command)))
    try:
        output = subprocess.check_output(command)
    except subprocess.CalledProcessError as e:
        print(e)
        sys.exit(1)
    sys.stdout.buffer.write(output)
    sys.stdout.flush()
    output = output.decode(errors="replace")
    return output.strip().splitlines()[-1]


# noinspection PyUnusedLocal
def get_requires_for_build_wheel(config_settings=None):
    if get_config().get("bindings") == "cffi":
        return ["cffi"]
    else:
        return []


# noinspection PyUnusedLocal
def get_requires_for_build_sdist(config_settings=None):
    return []


# noinspection PyUnusedLocal
def prepare_metadata_for_build_wheel(metadata_directory, config_settings=None):
    print("Checking for Rust toolchain....")
    is_cargo_installed = False
    try:
        output = subprocess.check_output(["cargo", "--version"]).decode(
            "utf-8", "ignore"
        )
        if "cargo" in output:
            is_cargo_installed = True
    except (FileNotFoundError, SubprocessError):
        pass

    if not is_cargo_installed:
        sys.stderr.write(
            "\nCargo, the Rust package manager, is not installed or is not on PATH.\n"
            "This package requires Rust and Cargo to compile extensions. Install it through\n"
            "the system's package manager or via https://rustup.rs/\n\n"
        )
        sys.exit(1)

    command = [
        "maturin",
        "pep517",
        "write-dist-info",
        "--metadata-directory",
        metadata_directory,
        # PEP 517 specifies that only `sys.executable` points to the correct
        # python interpreter
        "--interpreter",
        sys.executable,
    ]
    command.extend(get_config_options())

    print("Running `{}`".format(" ".join(command)))
    try:
        output = subprocess.check_output(command)
    except subprocess.CalledProcessError as e:
        print("Error: {}".format(e))
        sys.exit(1)
    sys.stdout.buffer.write(output)
    sys.stdout.flush()
    output = output.decode(errors="replace")
    return output.strip().splitlines()[-1]
