#!/usr/bin/env python3

import pyo3_mixed_py_subdir as pyo3_mixed


def test_get_42():
    assert pyo3_mixed.get_42() == 42
