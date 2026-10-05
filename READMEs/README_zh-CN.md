# LTBox

用 Rust 编写的桌面工具，用于刷写和修改受支持的联想平板固件。

[English](../README.md) · [한국어](README_ko-KR.md)

[![最新版本](https://img.shields.io/github/v/release/miner7222/LTBox)](https://github.com/miner7222/LTBox/releases/latest)
[![构建](https://img.shields.io/github/actions/workflow/status/miner7222/LTBox/rust-ci.yml?branch=main&label=build)](https://github.com/miner7222/LTBox/actions/workflows/rust-ci.yml)
[![许可证：GPLv3](https://img.shields.io/badge/License-GPLv3-blue.svg)](../LICENSE)
[![下载量](https://img.shields.io/github/downloads/miner7222/LTBox/total)](https://github.com/miner7222/LTBox/releases/latest)

LTBox 支持 **Windows、Linux 和 macOS**，提供固件刷写、区域转换、Root 和修复功能。可以按向导逐步操作，也可以使用高级工具单独处理各项任务。

**[下载](https://github.com/miner7222/LTBox/releases/latest) · [安装与使用指南（英文）](https://miner7222.github.io/ltbox/en/index.html) · [反馈问题](https://github.com/miner7222/LTBox/issues)**

## 使用前须知

> [!WARNING]
> 修改固件可能导致设备无法启动、数据丢失或保修失效。LTBox 仅供学习使用，不提供任何担保。使用风险由用户自行承担，开发者不承担责任。

- 刷写前请核对**准确的型号代码**、固件区域和对应设备的操作说明。
- 备份重要数据，并保留 LTBox 生成的备份。
- 开始前请阅读确认页面。

## 支持的设备

LTBox 支持 **TB320FC、TB321FU、TB322FC、TB323FU、TB324ZC、TB376FC、TB390FU、TB391FC、TB520FU 和 TB710FU**。

可用功能取决于型号、固件和连接模式。

| 设备 | 主要限制 |
| --- | --- |
| TB376FC / TB390FU / TB391FC | 不支持 Root、移除 Root、GPU 调整和启动修复。回滚信息仅供查看。 |
| TB323FU / TB324ZC | 不支持 GKI Root、启动修复和 AVB 区域转换。 |

## 安装与连接

从 [GitHub Releases](https://github.com/miner7222/LTBox/releases/latest) 下载发行包，或参阅[各平台安装说明（英文）](https://miner7222.github.io/ltbox/en/index.html)，了解包管理器安装方式和 USB 设置。

Windows 用户可以通过 Scoop 安装：

```powershell
scoop bucket add ltbox https://github.com/miner7222/scoop-bucket
scoop install ltbox
```

完成 USB 设置后，连接平板，在侧边栏选择任务并按向导操作。任务结束前请保持设备连接。

界面支持**英语、韩语、简体中文、俄语和日语**，可选择跟随系统、浅色或深色主题。

## 主要功能

| 任务 | 功能 |
| --- | --- |
| 刷写固件 | 根据型号处理区域和回滚信息，选择是否清除数据，然后准备并刷写固件。 |
| 获取或移除 Root | 使用所选 Root 方案修补启动镜像，或恢复备份的原厂镜像及相关验证元数据。 |
| 管理系统更新 | 停用或重新启用 OTA 更新；区域转换后因 OTA 无法启动时，可在受支持的设备上使用启动修复。 |
| 调整 GPU | 在受支持的设备上使用 KonaBess 修改频率和电压表，并重建相关 AVB 镜像。 |
| 查看信息与重启 | 查看设备信息，并根据当前连接状态重启至 Android、恢复模式、引导加载程序、fastbootd 或 EDL。 |

### Root 方案

支持 **Magisk、KernelSU、KernelSU Next、SukiSU Ultra、BakaSU、APatch、FolkPatch 和 SKRoot Lite**。可用性取决于设备和所选方案。

### 高级工具

- **区域与国家：**在 PRC 和 ROW 之间转换受支持的固件，或修改设备国家代码。
- **AVB 与回滚：**查看镜像元数据和回滚信息，修改受支持的回滚索引，或重建 vbmeta。
- **EDL 存储操作：**按名称读写分区，导出或刷写整个 LUN，或使用简易刷写工具刷写固件。
- **固件文件：**将 `.x` 文件解密为 rawprogram XML。

## 从源码构建

通过 [rustup 安装 Rust](https://rustup.rs/)，并准备对应平台的构建工具：Windows 需要 Visual Studio C++ Build Tools 和 Windows SDK；macOS 需要 Xcode Command Line Tools；Linux 需要 C/C++ 工具链及 [CI 工作流](../.github/workflows/rust-ci.yml)中列出的 USB、GUI 开发库。

Rust 工具链版本固定在 [rust-toolchain.toml](../rust-toolchain.toml) 中。

```sh
git clone https://github.com/miner7222/LTBox.git
cd LTBox
cargo build --release --locked -p ltbox-gui
```

生成的可执行文件为 `target/release/ltbox`，Windows 下为 `ltbox.exe`。

### 源码结构

| Crate | 职责 |
| --- | --- |
| [ltbox-core](../crates/ltbox-core) | 共用模型、设置、错误处理、日志、下载和固件工具。 |
| [ltbox-device](../crates/ltbox-device) | ADB、Fastboot、EDL/QDL 通信，设备发现和 USB 驱动集成。 |
| [ltbox-patch](../crates/ltbox-patch) | 启动与 AVB 镜像处理、区域转换、回滚处理和 Root 方案集成。 |
| [ltbox-gui](../crates/ltbox-gui) | iced 桌面应用和操作向导，生成 `ltbox` 可执行文件。 |

## 帮助与贡献

请先查阅[使用指南（英文）](https://miner7222.github.io/ltbox/en/index.html)。如问题仍未解决，请[提交 Issue](https://github.com/miner7222/LTBox/issues) 并提供：

- LTBox 版本和电脑操作系统。
- 准确的平板型号、固件版本与区域、连接模式。
- 复现步骤、预期行为和实际结果。
- 相关日志或截图，请先删除设备标识、个人路径和机密信息。不要附上 Root 密钥。

欢迎提交错误修复、设备相关发现、文档改进和翻译。界面翻译位于 [crates/ltbox-gui/lang](../crates/ltbox-gui/lang)，请保持各语言的翻译键和占位符一致。

修改代码后，请运行相关检查：

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

运行已忽略的测试或实际下载检查前，请确认所需条件。

LTBox 是个人业余项目，**不接受捐款、赞助或其他形式的资金支持**。

## 致谢

感谢以下朋友分享的发现、信息和指南，为 LTBox 的开发提供了帮助。

- **匿名用户 [ㅇㅇ](https://gall.dcinside.com/board/lists?id=tabletpc)**
- **[갓파더](https://ppomppu.co.kr/zboard/view.php?id=androidtab&page=1&divpage=38&no=197457)**
- **[limzei89](https://note.com/limzei89/n/nd5217eb57827)**
- **[hitin911](https://xdaforums.com/m/hitin911.12861404/)**
- **[corrupt3904](https://gall.dcinside.com/mgallery/board/view/?id=andtabcus&no=20290)**

## 许可证

LTBox 采用 [GPL-3.0-or-later](../LICENSE) 许可证。第三方组件遵循各自的许可证。

[![GPLv3](https://www.gnu.org/graphics/gplv3-127x51.png)](https://www.gnu.org/licenses/gpl-3.0)
