import os
from ltbox.utils import temporary_workspace

def test_temporary_workspace_creation_and_cleanup(tmp_path):
    target_dir = tmp_path / "temp_work"

    with temporary_workspace(target_dir) as workspace:
        assert workspace.exists()
        assert workspace.is_dir()

        (workspace / "test.txt").write_text("hello")

    assert not target_dir.exists()
