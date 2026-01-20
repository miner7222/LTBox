import sys
import os
import shutil
from unittest.mock import patch, MagicMock
import pytest

sys.path.append(os.path.abspath(os.path.join(os.path.dirname(__file__), '../bin')))

from ltbox.actions import xml as xml_action

@pytest.fixture
def mock_dirs(tmp_path):
    image_dir = tmp_path / "image"
    image_dir.mkdir()

    output_xml_dir = tmp_path / "output_xml"
    output_xml_dir.mkdir()

    with patch("ltbox.actions.xml.const.IMAGE_DIR", image_dir), \
         patch("ltbox.actions.xml.const.OUTPUT_XML_DIR", output_xml_dir):
        yield image_dir, output_xml_dir

class TestXmlDecryption:

    def test_decrypt_x_files_success(self, mock_dirs):
        image_dir, output_xml_dir = mock_dirs

        x_file = image_dir / "rawprogram0.x"
        x_file.write_text("encrypted_content")

        with patch("ltbox.actions.xml.utils.ui"), \
             patch("ltbox.actions.xml.utils.wait_for_directory"), \
             patch("ltbox.actions.xml.decrypt_file", return_value=True) as mock_decrypt:

            xml_action.decrypt_x_files()

            expected_src = str(x_file)
            expected_dst = str(output_xml_dir / "rawprogram0.xml")

            mock_decrypt.assert_called_with(expected_src, expected_dst)

    def test_existing_xml_files_move(self, mock_dirs):
        image_dir, output_xml_dir = mock_dirs

        xml_filename = "rawprogram0.xml"
        src_xml = image_dir / xml_filename
        src_xml.write_text("xml_content")

        with patch("ltbox.actions.xml.utils.ui"), \
             patch("ltbox.actions.xml.utils.wait_for_directory"):

            xml_action.decrypt_x_files()

            dest_xml = output_xml_dir / xml_filename
            assert dest_xml.exists()
            assert dest_xml.read_text() == "xml_content"
            assert not src_xml.exists()

    def test_no_files_found(self, mock_dirs):
        image_dir, output_xml_dir = mock_dirs

        with patch("ltbox.actions.xml.utils.ui"), \
             patch("ltbox.actions.xml.utils.wait_for_directory"):

            with pytest.raises(FileNotFoundError):
                xml_action.decrypt_x_files()

    def test_decrypt_failure_handling(self, mock_dirs):
        image_dir, output_xml_dir = mock_dirs
        (image_dir / "bad_file.x").write_text("corrupted")

        with patch("ltbox.actions.xml.utils.ui") as mock_ui, \
             patch("ltbox.actions.xml.utils.wait_for_directory"), \
             patch("ltbox.actions.xml.decrypt_file", return_value=False):

            with pytest.raises(FileNotFoundError):
                xml_action.decrypt_x_files()

            assert mock_ui.info.called
