# LTBox

A desktop tool for flashing and modifying firmware on supported Lenovo tablets, written in Rust.

[한국어](READMEs/README_ko-KR.md) · [简体中文](READMEs/README_zh-CN.md)

[![Latest release](https://img.shields.io/github/v/release/miner7222/LTBox)](https://github.com/miner7222/LTBox/releases/latest)
[![Build](https://img.shields.io/github/actions/workflow/status/miner7222/LTBox/rust-ci.yml?branch=main&label=build)](https://github.com/miner7222/LTBox/actions/workflows/rust-ci.yml)
[![License: GPLv3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)
[![Downloads](https://img.shields.io/github/downloads/miner7222/LTBox/total)](https://github.com/miner7222/LTBox/releases/latest)

LTBox brings firmware flashing, region conversion, rooting, and recovery tools into a native GUI for **Windows, Linux, and macOS**, with guided workflows and advanced tools for individual operations.

**[Download](https://github.com/miner7222/LTBox/releases/latest) · [Installation and user guide](https://miner7222.github.io/ltbox/en/index.html) · [Report an issue](https://github.com/miner7222/LTBox/issues)**

## Before you start

> [!WARNING]
> Modifying firmware can make your device unbootable, erase data, or void your warranty. LTBox is provided for educational purposes, without warranty. You use it at your own risk; the developer assumes no liability.

- Check the **exact model code**, firmware region, and instructions for your device before flashing.
- Back up important data and keep any backups created by LTBox.
- Read the confirmation screen before starting.

## Supported devices

LTBox supports **TB320FC, TB321FU, TB322FC, TB323FU, TB324ZC, TB376FC, TB390FU, TB391FC, TB520FU, and TB710FU**.

Features depend on the model, firmware, and connection mode:

| Device group | Important limitations |
| --- | --- |
| TB376FC / TB390FU / TB391FC | Root, unroot, GPU tuning, and boot rescue are unavailable. Rollback information is read-only. |
| TB323FU / TB324ZC | GKI rooting, boot rescue, and AVB region conversion are unavailable. |

## Install and connect

Get a packaged build from [GitHub Releases](https://github.com/miner7222/LTBox/releases/latest), or follow the [platform-specific installation instructions](https://miner7222.github.io/ltbox/en/index.html) for package managers and USB setup.

On Windows, Scoop is available:

```powershell
scoop bucket add ltbox https://github.com/miner7222/scoop-bucket
scoop install ltbox
```

After USB setup, connect your tablet, choose a task from the sidebar, and follow the wizard. Keep the device connected until the operation finishes.

The interface supports **English, Korean, Simplified Chinese, Russian, Japanese, and French**, with system, light, and dark themes.

## What you can do

| Task | Capabilities |
| --- | --- |
| Flash firmware | Prepare and flash firmware, with model-specific region and rollback handling and data-wipe choices. |
| Root and unroot | Patch supported boot images with a selected root provider; restore backed-up stock images and associated verification metadata. |
| Manage system updates | Disable or re-enable OTA updates; use boot recovery on supported devices after a region-converted OTA fails to boot. |
| Debloat | Remove or restore preinstalled apps for the current user from a reviewed per-model list (TB321FU for now). |
| Tune the GPU | Edit clock and voltage tables with KonaBess and rebuild the affected AVB images on supported models. |
| Inspect and reboot | View device information and reboot into Android, recovery, bootloader, fastbootd, or EDL as supported by the current connection. |

### Root providers

Supported integrations include **Magisk, KernelSU, KernelSU Next, SukiSU Ultra, ReSukiSU, APatch, FolkPatch, and SKRoot Lite**. Availability depends on the device and provider.

### Advanced tools

- **Region and country:** convert supported firmware between PRC and ROW, or change the device country code.
- **AVB and rollback:** inspect image metadata and rollback information, modify supported rollback indices, and rebuild vbmeta.
- **EDL storage:** read or write named partitions, dump or flash whole LUNs, and flash firmware through the simple flasher.
- **Firmware files:** decrypt `.x` files into rawprogram XML.

## Build from source

Install [Rust through rustup](https://rustup.rs/) and native build tools: Visual Studio C++ Build Tools and the Windows SDK on Windows, Xcode Command Line Tools on macOS, or a C/C++ toolchain and the USB/GUI development libraries listed in the [CI workflow](.github/workflows/rust-ci.yml) on Linux.

The repository pins its Rust toolchain in [rust-toolchain.toml](rust-toolchain.toml).

```sh
git clone https://github.com/miner7222/LTBox.git
cd LTBox
cargo build --release --locked -p ltbox-gui
```

The executable is `target/release/ltbox` (`ltbox.exe` on Windows).

### Source layout

| Crate | Responsibility |
| --- | --- |
| [ltbox-core](crates/ltbox-core) | Shared models, settings, errors, logging, downloads, and firmware utilities. |
| [ltbox-device](crates/ltbox-device) | ADB, Fastboot, EDL/QDL transport, device discovery, and USB driver integration. |
| [ltbox-patch](crates/ltbox-patch) | Boot and AVB image processing, region conversion, rollback handling, and root providers. |
| [ltbox-gui](crates/ltbox-gui) | The iced desktop application and guided workflows; produces the `ltbox` executable. |

## Help and contributions

Start with the [user guide](https://miner7222.github.io/ltbox/en/index.html). If a problem remains, [open an issue](https://github.com/miner7222/LTBox/issues) with:

- LTBox version and host operating system.
- Exact tablet model, firmware version and region, and connection mode.
- Steps to reproduce the problem, expected behavior, and actual result.
- Relevant logs or screenshots, with device identifiers, personal paths, and secrets removed. Do not attach root keys.

Bug fixes, device findings, documentation improvements, and translations are welcome. UI translations live in [crates/ltbox-gui/lang](crates/ltbox-gui/lang); keep translation keys and placeholders consistent across languages.

For code changes, run the relevant checks:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Check prerequisites before running ignored tests or live download checks.

LTBox is a personal hobby project and does **not** accept donations, sponsorships, or other financial support.

## Credits

Thanks to the following people for sharing findings, information, and guides that helped shape LTBox.

- **Anonymous [ㅇㅇ](https://gall.dcinside.com/board/lists?id=tabletpc)**
- **[갓파더](https://ppomppu.co.kr/zboard/view.php?id=androidtab&page=1&divpage=38&no=197457)**
- **[limzei89](https://note.com/limzei89/n/nd5217eb57827)**
- **[hitin911](https://xdaforums.com/m/hitin911.12861404/)**
- **[corrupt3904](https://gall.dcinside.com/mgallery/board/view/?id=andtabcus&no=20290)**

## License

LTBox is licensed under [GPL-3.0-or-later](LICENSE). Third-party components retain their respective licenses.

[![GPLv3](https://www.gnu.org/graphics/gplv3-127x51.png)](https://www.gnu.org/licenses/gpl-3.0)
