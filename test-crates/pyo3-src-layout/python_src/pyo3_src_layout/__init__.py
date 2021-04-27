from .my_submodule.double import double
from .pyo3_src_layout import get_21


def get_42() -> int:
    return double(get_21)
