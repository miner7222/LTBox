import logging
import sys
from contextlib import contextmanager
from typing import Optional

LOGGER_NAME = "ltbox"
_logger = logging.getLogger(LOGGER_NAME)
_logger.setLevel(logging.INFO)

if not _logger.handlers:
    console_handler = logging.StreamHandler(sys.stdout)
    console_handler.setFormatter(logging.Formatter('%(message)s'))
    _logger.addHandler(console_handler)

def get_logger() -> logging.Logger:
    return _logger

@contextmanager
def logging_context(log_filename: Optional[str] = None):
    handlers_to_remove = []
    
    try:
        if log_filename:
            file_handler = logging.FileHandler(log_filename, encoding='utf-8')
            file_handler.setFormatter(logging.Formatter('%(asctime)s - %(message)s', datefmt='%H:%M:%S'))
            _logger.addHandler(file_handler)
            handlers_to_remove.append(file_handler)
        
        yield _logger

    finally:
        for handler in handlers_to_remove:
            handler.close()
            _logger.removeHandler(handler)