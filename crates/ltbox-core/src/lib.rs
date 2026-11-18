//! `ltbox-core` — domain layer shared across LTBox crates.
//!
//! Model capabilities, firmware-file helpers (`.x` decryption, XML parsing),
//! network clients (GitHub, Lenovo), i18n and the live log sink. Every
//! fallible API returns [`Result<T>`] / [`LtboxError`].

pub mod app_paths;
pub mod connectivity;
pub mod crypto;
pub mod downloader;
pub mod error;
pub mod github;
pub mod i18n;
pub mod install_source;
pub mod lenovo_info;
pub mod lenovo_ota;
pub mod lenovo_qfil;
pub mod live_sink;
pub mod log_format;
pub mod model;
pub mod obf;
pub mod partition_lun;
pub mod runtime;
pub mod safe_path;
pub mod sahara_xml;
pub mod xml;

pub use error::{LtboxError, Result};

/// Send a line to the in-process sink and the caller's `&mut Vec<String>`.
/// Echo to stdout only without a GUI consumer, avoiding a second copy through
/// its native pipe tap. Don't re-extend the closure's Vec post-flow.
#[macro_export]
macro_rules! live {
    ($log:expr, $($arg:tt)*) => {{
        let _line = format!($($arg)*);
        $crate::live_sink::emit($crate::live_sink::Entry::info(_line.clone()));
        $log.push(_line);
    }};
}

/// Emit diagnostic detail. Saved in the full log, hidden in the normal GUI log.
#[macro_export]
macro_rules! live_debug {
    ($log:expr, $($arg:tt)*) => {{
        let _line = format!($($arg)*);
        $crate::live_sink::emit($crate::live_sink::Entry::debug(_line.clone()));
        $log.push(format!("[Debug] {}", _line));
    }};
}

/// Translate `$key` and substitute `{name}` placeholders from `name = value`
/// pairs. Each value is formatted with `Display`; pre-format floats yourself
/// (`format!("{x:.1}")`).
///
/// ```ignore
/// live!(
///     log,
///     "[Driver] {}",
///     tr_args!(
///         "live_driver_progress_pct",
///         name = display_name,
///         pct = format!("{pct:>3}"),
///         downloaded = format!("{dl_mb:.1}"),
///     )
/// );
/// ```
#[macro_export]
macro_rules! tr_args {
    ($key:expr $(, $name:ident = $val:expr)* $(,)?) => {{
        $crate::i18n::format_template(
            &$crate::i18n::tr($key),
            &[$((stringify!($name), format!("{}", $val))),*],
        )
    }};
}
