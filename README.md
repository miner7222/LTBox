# LTBox

## ⚠️ Important: Disclaimer

**This project is for educational purposes ONLY.**

Modifying your device's boot images carries significant risks, including but not limited to, bricking your device, data loss, or voiding your warranty. The author **assumes no liability** and is not responsible for any **damage or consequence** that may occur to **your device or anyone else's device** from using these scripts.

**You are solely responsible for any consequences. Use at your own absolute risk.**

---

## 1. Core Vulnerability & Overview

This toolkit exploits a security vulnerability found in certain Lenovo Android tablets. These devices have firmware signed with publicly available **AOSP (Android Open Source Project) test keys**.

Because of this vulnerability, the device's bootloader trusts and boots any image signed with these common test keys, even if the bootloader is **locked**.

This toolkit is an all-in-one collection of scripts that leverages this flaw to perform advanced modifications on a device with a locked bootloader.

### Target Models

* Lenovo Legion Y700 (2nd, 3rd, 4th Gen)
* Lenovo Tab Plus AI (AKA Yoga Pad Pro AI)
* Lenovo Xiaoxin Pad Pro GT

*...Other recent Lenovo tablets (those released in 2024 or later with Qualcomm chipsets) may also be vulnerable.*

## 2. Toolkit Purpose & Features

This toolkit provides an all-in-one solution for the following tasks **without unlocking the bootloader**:

1.  **Region Conversion (PRC → ROW)**
    * Converts the region code in `vendor_boot.img` to allow flashing a global (ROW) ROM on a Chinese (PRC) model.
    * Re-makes the `vbmeta.img` with the AOSP test keys to validate the modified `vendor_boot`.
2.  **Rooting (via KernelSU)**
    * Patches the stock `boot.img` by replacing the original kernel with [one that includes KernelSU](https://github.com/WildKernels/GKI_KernelSU_SUSFS).
    * Re-signs the patched `boot.img` with AOSP test keys.
3.  **Region Code Reset**
    * Modifies byte patterns in `devinfo.img` and `persist.img` to reset region-lock settings.
4.  **EDL Partition Read/Write**
    * Uses `edl-ng` to read (dump) the `devinfo` and `persist` partitions directly from the device in EDL mode.
    * Writes (flashes) the patched `devinfo.img` and `persist.img` back to the device in EDL mode.
5.  **Anti-Rollback (ARB) Bypass**
    * Patches firmware images (e.g., `boot.img`, `vbmeta_system.img`) that you intend to flash (e.g., for a downgrade).
    * It reads the rollback index from your *currently installed* firmware and forcibly applies that same (higher) index to the *new* firmware, bypassing Anti-Rollback Protection.

## 3. Prerequisites

Before you begin, place the required stock firmware images into the appropriate folders. All necessary tools (Python, avbtool, etc.) will be downloaded automatically by the scripts.

* **For Region Conversion:** Place `vendor_boot.img` and `vbmeta.img` in the main folder.
* **For Rooting:** Place `boot.img` in the main folder.
* **For Region Reset:** Place `devinfo.img` and `persist.img` in the main folder (or use `read_edl.bat` to dump them).
* **For Anti-Rollback Bypass:**
    * `input_current` folder: Place `boot.img` and `vbmeta_system.img` from your **currently installed** firmware.
    * `input_new` folder: Place the `boot.img` and `vbmeta_system.img` from the **new (downgrade) firmware** you wish to flash.

## 4. How to Use

1.  **Place Images:** Put the necessary `.img` files into the correct folder (see section 3) according to the task you want to perform.
2.  **Run the Script:** Simply double-click the `.bat` file corresponding to the task you want to perform.
3.  **Get Results:** After the script finishes, the modified images will be saved in a corresponding `output*` folder (e.g., `output`, `output_root`).
4.  **Flash the Images:** Flash the new `.img` file(s) from the output folder to your device using `fastboot` or an EDL tool.

## 5. Script Descriptions

* **`vndrboot_vbmeta.bat`**: Handles the `vendor_boot` region conversion (PRC→ROW) and remakes `vbmeta.img`.
    * *Output: `output` folder*
* **`root.bat`**: Patches `boot.img` with KernelSU for root access.
    * *Output: `output_root` folder*
* **`devinfo_persist.bat`**: Modifies `devinfo.img` and `persist.img` to reset region code.
    * *Output: `output_dp` folder*
* **`anti-antirollback.bat`**: Bypasses Anti-Rollback Protection by patching downgrade firmware images.
    * *Requires: Files in `input_current` and `input_new` folders.*
    * *Output: `output_anti_rollback` folder*
* **`read_edl.bat`**: Dumps `devinfo.img` and `persist.img` from your device via EDL mode.
    * *Output: Main folder*
* **`write_edl.bat`**: Flashes the modified images from the `output_dp` folder to your device via EDL mode.
* **`info_image.bat`**: Drag & drop `.img` file(s) or folder(s) onto this script to see AVB (Android Verified Boot) information.
    * *Output: `image_info_*.txt`*