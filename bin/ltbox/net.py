import time
from contextlib import contextmanager
from typing import Dict, Generator, Optional

import requests


@contextmanager
def request_with_retries(
    method: str,
    url: str,
    *,
    headers: Optional[Dict[str, str]] = None,
    timeout: int = 30,
    retries: int = 3,
    backoff: float = 5,
    stream: bool = True,
    allow_redirects: bool = True,
) -> Generator[requests.Response, None, None]:
    for attempt in range(retries + 1):
        try:
            response = requests.request(
                method,
                url,
                headers=headers,
                timeout=timeout,
                stream=stream,
                allow_redirects=allow_redirects,
            )
            response.raise_for_status()
            with response:
                yield response
            return
        except requests.RequestException:
            if attempt >= retries:
                raise
            time.sleep(backoff * (attempt + 1))
