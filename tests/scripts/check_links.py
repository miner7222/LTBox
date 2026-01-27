import json
import sys
from pathlib import Path

import requests


def check_url(url: str, description: str) -> bool:
    print(f"Checking {description}...", end=" ")
    try:
        response = requests.get(url, stream=True, timeout=15)
        if response.status_code in [200, 302]:
            print(f"OK ({url})")
            return True
        print(f"FAILED (Status: {response.status_code}) - {url}")
        return False
    except Exception as exc:
        print(f"ERROR - {url} ({exc})")
        return False


def check_github_api(owner_repo: str, tag: str, description: str) -> bool:
    if "github.com/" in owner_repo:
        owner_repo = owner_repo.split("github.com/")[-1]

    if not tag or tag == "latest":
        url = f"https://api.github.com/repos/{owner_repo}/releases/latest"
    else:
        url = f"https://api.github.com/repos/{owner_repo}/releases/tags/{tag}"

    return check_url(url, f"GitHub API ({description})")


def main() -> None:
    config_path = Path("bin/ltbox/config.json")
    if not config_path.exists():
        print("::error::Config file not found!")
        sys.exit(1)

    with open(config_path, "r", encoding="utf-8") as f:
        config = json.load(f)

    has_error = False

    # 1. Static Tools
    tools = config.get("tools", {})
    print("--- Static Tools ---")
    if not check_url(tools.get("platform_tools_url"), "Platform Tools"):
        has_error = True
    if not check_url(tools.get("avb_archive_url"), "AVB Archive"):
        has_error = True
    if not check_url(tools.get("openssl_url"), "OpenSSL"):
        has_error = True

    # 2. Magiskboot (GitHub Release)
    print("\n--- Magiskboot ---")
    mb = config.get("magiskboot", {})
    mb_repo = mb.get("repo") or mb.get("repo_url")
    if not check_github_api(mb_repo, mb.get("tag"), "Magiskboot Release"):
        has_error = True

    # 3. KernelSU-Next (GitHub Release)
    print("\n--- KernelSU-Next ---")
    ksu = config.get("kernelsu-next", {})
    ksu_repo = ksu.get("repo") or ksu.get("apk_repo")
    ksu_tag = ksu.get("tag") or ksu.get("apk_tag")

    # 3-1. Release API Check
    if not check_github_api(ksu_repo, ksu_tag, "KernelSU-Next Release"):
        has_error = True

    # 3-2. KSUInit (Raw)
    ksuinit_url = (
        f"https://github.com/{ksu_repo}/raw/refs/tags/"
        f"{ksu_tag}/userspace/ksud/bin/aarch64/ksuinit"
    )
    if not check_url(ksuinit_url, "KSUInit Binary"):
        has_error = True

    # 3-3. KernelSU Next (Nightly)
    nightly_wf = ksu.get("nightly_workflow")
    nightly_mgr = ksu.get("nightly_manager")
    if nightly_wf and nightly_mgr:
        url = f"https://nightly.link/{ksu_repo}/actions/runs/{nightly_wf}/{nightly_mgr}"
        if not check_url(url, "KernelSU-Next Nightly"):
            has_error = True

    # 4. GKI_KernelSU_SUSFS
    print("\n--- WildKernels ---")
    wk = config.get("wildkernels", {})
    wk_owner = wk.get("owner", "WildKernels")
    wk_repo = wk.get("repo", "GKI_KernelSU_SUSFS")
    wk_tag = wk.get("tag", "latest")
    if not check_github_api(f"{wk_owner}/{wk_repo}", wk_tag, "WildKernels GKI"):
        has_error = True

    # 5. SukiSU Ultra (Nightly)
    print("\n--- SukiSU Ultra ---")
    suki = config.get("sukisu-ultra", {})
    suki_repo = suki.get("repo")
    suki_wf = suki.get("workflow")
    suki_mgr = suki.get("manager")
    if suki_repo and suki_wf and suki_mgr:
        url = f"https://nightly.link/{suki_repo}/actions/runs/{suki_wf}/{suki_mgr}"
        if not check_url(url, "SukiSU Nightly"):
            has_error = True

    if has_error:
        sys.exit(1)


if __name__ == "__main__":
    main()
