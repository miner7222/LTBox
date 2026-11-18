//! Non-Windows, non-Linux hosts (macOS): no driver or udev rules are needed,
//! since userspace USB claims the EDL device directly. Probes are no-ops, so
//! the GUI's driver banner never fires; kernel mode reports unsupported.

use super::{DriverError, DriverStatus, DriverUpdate, Result, qcom_driver_mode};

pub fn check_required_drivers() -> DriverStatus {
    if qcom_driver_mode().is_kernel() {
        DriverStatus::KernelDriverUnsupported
    } else {
        DriverStatus::NotWindows
    }
}

pub fn check_driver_update() -> Option<DriverUpdate> {
    None
}

pub fn download_and_install(_log: &mut Vec<String>) -> Result<()> {
    Err(DriverError::NotWindows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_check_is_not_windows() {
        assert_eq!(check_required_drivers(), DriverStatus::NotWindows);
    }

    #[test]
    fn unsupported_install_errors_with_not_windows() {
        let mut log = Vec::new();
        assert!(matches!(
            download_and_install(&mut log).unwrap_err(),
            DriverError::NotWindows
        ));
    }
}
