from typing import Callable


def double(fn: Callable[[], int]) -> int:
    return 2 * fn()
