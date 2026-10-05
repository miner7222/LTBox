# LTBox

지원되는 Lenovo 태블릿의 펌웨어를 플래싱하고 수정하는 Rust 기반 데스크톱 도구입니다.

[English](../README.md) · [简体中文](README_zh-CN.md)

[![최신 릴리스](https://img.shields.io/github/v/release/miner7222/LTBox)](https://github.com/miner7222/LTBox/releases/latest)
[![빌드](https://img.shields.io/github/actions/workflow/status/miner7222/LTBox/rust-ci.yml?branch=main&label=build)](https://github.com/miner7222/LTBox/actions/workflows/rust-ci.yml)
[![라이선스: GPLv3](https://img.shields.io/badge/License-GPLv3-blue.svg)](../LICENSE)
[![다운로드](https://img.shields.io/github/downloads/miner7222/LTBox/total)](https://github.com/miner7222/LTBox/releases/latest)

LTBox는 **Windows, Linux, macOS**에서 펌웨어 플래싱, 지역 변환, 루팅, 복구 기능을 제공합니다. 단계별 위저드로 작업을 진행하거나 고급 도구로 개별 작업을 수행할 수 있습니다.

**[다운로드](https://github.com/miner7222/LTBox/releases/latest) · [설치 및 사용 가이드](https://miner7222.github.io/ltbox/ko/index.html) · [문제 제보](https://github.com/miner7222/LTBox/issues)**

## 시작하기 전에

> [!WARNING]
> 펌웨어를 수정하면 기기가 부팅되지 않거나 데이터가 삭제되고 보증이 무효화될 수 있습니다. LTBox는 교육 목적으로 제공되며 어떠한 보증도 하지 않습니다. 사용에 따른 위험과 책임은 사용자에게 있으며, 개발자는 책임을 지지 않습니다.

- 플래싱 전에 **정확한 모델 코드**, 펌웨어 지역, 기기별 안내를 확인하세요.
- 중요한 데이터를 백업하고 LTBox가 생성한 백업도 보관하세요.
- 작업을 시작하기 전에 확인 화면을 읽어보세요.

## 지원 기기

LTBox는 **TB320FC, TB321FU, TB322FC, TB323FU, TB324ZC, TB376FC, TB390FU, TB391FC, TB520FU, TB710FU**를 지원합니다.

사용할 수 있는 기능은 모델, 펌웨어, 연결 모드에 따라 다릅니다.

| 기기 | 주요 제한 사항 |
| --- | --- |
| TB376FC / TB390FU / TB391FC | 루팅, 루팅 해제, GPU 조정, 부팅 복구를 지원하지 않습니다. 롤백 정보는 조회만 가능합니다. |
| TB323FU / TB324ZC | GKI 루팅, 부팅 복구, AVB 지역 변환을 지원하지 않습니다. |

## 설치 및 연결

[GitHub Releases](https://github.com/miner7222/LTBox/releases/latest)에서 배포 파일을 받거나, [운영체제별 설치 안내](https://miner7222.github.io/ltbox/ko/index.html)에서 패키지 관리자 설치 방법과 USB 설정을 확인하세요.

Windows에서는 Scoop으로 설치할 수 있습니다.

```powershell
scoop bucket add ltbox https://github.com/miner7222/scoop-bucket
scoop install ltbox
```

USB 설정을 마친 뒤 태블릿을 연결하고 사이드바에서 작업을 선택해 위저드를 따라 진행하세요. 작업이 끝날 때까지 기기 연결을 유지하세요.

인터페이스는 **영어, 한국어, 중국어 간체, 러시아어, 일본어**와 시스템 설정에 따른 테마, 밝은 테마, 어두운 테마를 지원합니다.

## 주요 기능

| 작업 | 기능 |
| --- | --- |
| 펌웨어 플래싱 | 모델에 맞는 지역·롤백 처리와 데이터 삭제 옵션을 적용해 펌웨어를 준비하고 플래싱합니다. |
| 루팅 및 루팅 해제 | 선택한 루트 제공자로 부트 이미지를 패치하거나, 백업한 순정 이미지와 검증 메타데이터를 복원합니다. |
| 시스템 업데이트 관리 | OTA 업데이트를 끄거나 다시 켭니다. 지원 기기에서는 지역 변환 후 OTA로 부팅되지 않을 때 부팅 복구를 사용할 수 있습니다. |
| GPU 조정 | 지원 기기에서 KonaBess로 클럭·전압 테이블을 수정하고 관련 AVB 이미지를 재구성합니다. |
| 기기 정보 및 재부팅 | 기기 정보를 확인하고 현재 연결에서 지원하는 Android, 복구, 부트로더, fastbootd, EDL 모드로 재부팅합니다. |

### 루트 제공자

**Magisk, KernelSU, KernelSU Next, SukiSU Ultra, BakaSU, APatch, FolkPatch, SKRoot Lite**를 지원합니다. 사용 가능 여부는 기기와 제공자에 따라 다릅니다.

### 고급 도구

- **지역 및 국가:** 지원 펌웨어의 PRC·ROW 지역을 변환하거나 기기의 국가 코드를 변경합니다.
- **AVB 및 롤백:** 이미지 메타데이터와 롤백 정보를 확인하고, 지원되는 롤백 인덱스를 수정하거나 vbmeta를 재구성합니다.
- **EDL 저장소:** 이름으로 파티션을 읽고 쓰거나, LUN 전체를 덤프·플래싱하고, 간단 플래셔로 펌웨어를 플래싱합니다.
- **펌웨어 파일:** `.x` 파일을 복호화해 rawprogram XML로 변환합니다.

## 소스에서 빌드

[rustup으로 Rust](https://rustup.rs/)를 설치하고 운영체제에 맞는 빌드 도구를 준비하세요. Windows는 Visual Studio C++ Build Tools와 Windows SDK, macOS는 Xcode Command Line Tools, Linux는 C/C++ 도구 모음과 [CI 워크플로](../.github/workflows/rust-ci.yml)에 명시된 USB·GUI 개발 라이브러리가 필요합니다.

Rust 도구 모음 버전은 [rust-toolchain.toml](../rust-toolchain.toml)에 고정되어 있습니다.

```sh
git clone https://github.com/miner7222/LTBox.git
cd LTBox
cargo build --release --locked -p ltbox-gui
```

실행 파일은 `target/release/ltbox`에 생성됩니다. Windows에서는 `ltbox.exe`입니다.

### 소스 구조

| 크레이트 | 역할 |
| --- | --- |
| [ltbox-core](../crates/ltbox-core) | 공통 모델, 설정, 오류, 로깅, 다운로드, 펌웨어 유틸리티 |
| [ltbox-device](../crates/ltbox-device) | ADB, Fastboot, EDL/QDL 통신, 기기 탐색, USB 드라이버 연동 |
| [ltbox-patch](../crates/ltbox-patch) | 부트·AVB 이미지 처리, 지역 변환, 롤백 처리, 루트 제공자 연동 |
| [ltbox-gui](../crates/ltbox-gui) | iced 데스크톱 앱과 위저드. `ltbox` 실행 파일 생성 |

## 도움말 및 기여

먼저 [사용 가이드](https://miner7222.github.io/ltbox/ko/index.html)를 확인하세요. 문제가 해결되지 않으면 다음 정보를 포함해 [이슈를 등록](https://github.com/miner7222/LTBox/issues)해 주세요.

- LTBox 버전과 PC 운영체제
- 정확한 태블릿 모델, 펌웨어 버전·지역, 연결 모드
- 재현 단계, 기대한 동작, 실제 결과
- 기기 식별자, 개인 경로, 비밀 정보를 제거한 로그나 스크린샷. 루트 키는 첨부하지 마세요.

버그 수정, 기기 관련 발견, 문서 개선, 번역 기여를 환영합니다. UI 번역은 [crates/ltbox-gui/lang](../crates/ltbox-gui/lang)에 있으며, 언어 간 번역 키와 치환 변수를 일치시켜 주세요.

코드를 변경했다면 관련 검사를 실행하세요.

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

무시된 테스트나 실제 다운로드 검사를 실행하기 전에는 필요한 조건을 확인하세요.

LTBox는 개인 취미 프로젝트이며 **기부, 스폰서십을 비롯한 금전적 후원을 받지 않습니다.**

## 감사의 말

LTBox 개발에 도움이 된 발견, 정보, 가이드를 공유해 주신 다음 분들께 감사드립니다.

- **익명 [ㅇㅇ](https://gall.dcinside.com/board/lists?id=tabletpc)**
- **[갓파더](https://ppomppu.co.kr/zboard/view.php?id=androidtab&page=1&divpage=38&no=197457)**
- **[limzei89](https://note.com/limzei89/n/nd5217eb57827)**
- **[hitin911](https://xdaforums.com/m/hitin911.12861404/)**
- **[corrupt3904](https://gall.dcinside.com/mgallery/board/view/?id=andtabcus&no=20290)**

## 라이선스

LTBox는 [GPL-3.0-or-later](../LICENSE)로 배포됩니다. 타사 구성 요소에는 각각의 라이선스가 적용됩니다.

[![GPLv3](https://www.gnu.org/graphics/gplv3-127x51.png)](https://www.gnu.org/licenses/gpl-3.0)
