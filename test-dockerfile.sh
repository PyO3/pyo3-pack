#!/usr/bin/env bash
# Builds all 5 test crates using the docker container,
# installs the wheel and checks that the installed package is functional

set -e

rm -rf venv-docker

python3.6 -m venv venv-docker

source venv-docker/bin/activate

pip install cffi > /dev/null

docker run --rm -v $(pwd)/test-crates/hello-world:/io maturin build --no-sdist -b bin

pip install hello-world --no-index --find-links test-crates/hello-world/target/wheels/

if [[ $(python test-crates/hello-world/check_installed/check_installed.py) != 'SUCCESS' ]]; then
  exit 1
fi

docker run --rm -v $(pwd)/test-crates/cffi-pure:/io maturin build --no-sdist -b cffi

pip install cffi-pure --no-index --find-links test-crates/cffi-pure/target/wheels/

if [[ $(python test-crates/cffi-pure/check_installed/check_installed.py) != 'SUCCESS' ]]; then
  exit 1
fi

docker run --rm -v $(pwd)/test-crates/cffi-mixed:/io maturin build --no-sdist -b cffi

pip install cffi-mixed --no-index --find-links test-crates/cffi-mixed/target/wheels/

if [[ $(python test-crates/cffi-mixed/check_installed/check_installed.py) != 'SUCCESS' ]]; then
  exit 1
fi

docker run --rm -v $(pwd)/test-crates/pyo3-pure:/io maturin build --no-sdist -i python3.6

pip install pyo3-pure --no-index --find-links test-crates/pyo3-pure/target/wheels/

if [[ $(python test-crates/pyo3-pure/check_installed/check_installed.py) != 'SUCCESS' ]]; then
  exit 1
fi

docker run --rm -v $(pwd)/test-crates/pyo3-mixed:/io maturin build --no-sdist -i python3.6 --py-src python

pip install pyo3-mixed --no-index --find-links test-crates/pyo3-mixed/target/wheels/

if [[ $(python test-crates/pyo3-mixed/check_installed/check_installed.py) != 'SUCCESS' ]]; then
  exit 1
fi

deactivate
