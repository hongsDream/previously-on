TOTAL_BUDGET_SECONDS = 30.0


def selected_delay(attempt, retry_after=None):
    """Return the pre-challenge per-attempt delay without cumulative clamping."""
    if retry_after is not None:
        try:
            parsed = float(retry_after)
        except (TypeError, ValueError):
            parsed = -1.0
        if parsed >= 0:
            return parsed
    return min(2.0 ** attempt, 10.0)
