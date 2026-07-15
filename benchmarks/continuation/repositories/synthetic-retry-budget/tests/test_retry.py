from client.retry import selected_delay


def test_valid_retry_after_wins():
    assert selected_delay(2, "7") == 7.0


def test_invalid_and_negative_retry_after_use_fallback():
    assert selected_delay(2, "invalid") == 4.0
    assert selected_delay(2, -1) == 4.0
