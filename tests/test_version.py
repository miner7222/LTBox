import pytest
from ltbox.utils import is_update_available

@pytest.mark.parametrize("current, latest, expected", [
    ("v1.0.0", "v1.0.1", True),
    ("v1.0.1", "v1.0.0", False),
    ("v1.0.0", "v1.0.0", False),
    ("v1.2", "v1.2.1", True),
    ("0.0.1", "0.0.2", True),
])
def test_is_update_available(current, latest, expected):
    assert is_update_available(current, latest) == expected
