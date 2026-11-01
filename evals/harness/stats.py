"""Small statistics for pass rates."""
import math


def wilson(passed, n, z=1.96):
    """The Wilson score interval of a pass rate, as (rate, low, high)."""
    if n == 0:
        return (None, None, None)
    p = passed / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return (p, max(0.0, c - h), min(1.0, c + h))


def fmt_rate(passed, n):
    p, lo, hi = wilson(passed, n)
    if p is None:
        return "n/a"
    return f"{100 * p:.0f}% [{100 * lo:.0f}–{100 * hi:.0f}] (n={n})"


def mean(xs):
    xs = [x for x in xs if x is not None]
    return sum(xs) / len(xs) if xs else None


def stdev(xs):
    xs = [x for x in xs if x is not None]
    if len(xs) < 2:
        return 0.0
    m = mean(xs)
    return math.sqrt(sum((x - m) ** 2 for x in xs) / (len(xs) - 1))
