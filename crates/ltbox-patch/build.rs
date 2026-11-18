// Windows' installer-detection heuristic auto-elevates any EXE whose name
// contains `setup`, `install`, `update` or `patch`. The lib-test binary
// `ltbox_patch-<hash>.exe` trips it and `cargo test` fails with `os error 740`
// under non-elevated shells.
//
// Embed a `requestedExecutionLevel level="asInvoker"` manifest into every EXE
// this crate links (in practice only the lib-test harness) so Windows skips
// the heuristic. MSVC-only; MinGW / non-Windows hosts ignore manifests.
use std::fs;
use std::path::PathBuf;

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_os != "windows" || target_env != "msvc" {
        return;
    }
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let manifest_path = out_dir.join("ltbox_patch_asinvoker.manifest");
    let manifest_xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;
    fs::write(&manifest_path, manifest_xml).expect("write manifest");
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
        manifest_path.display()
    );
    println!("cargo:rerun-if-changed=build.rs");
}
