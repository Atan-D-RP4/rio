#![cfg(target_os = "macos")]

pub mod appkit;

use core::fmt;

/// Error that can occur during operation of `menubar`.
pub struct Error(Box<Impl>);

enum Impl {
    /// Standard input/output error.
    #[allow(unused)]
    Io(std::io::Error),

    /// A menu already exists.
    #[allow(unused)]
    MenuExists,

    /// This isn't the window type we expected.
    #[allow(unused)]
    UnexpectedWindowType,
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &*self.0 {
            Impl::Io(io) => fmt::Debug::fmt(io, f),
            Impl::MenuExists => f.write_str("MenuExists"),
            Impl::UnexpectedWindowType => f.write_str("UnexpectedWindowType"),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &*self.0 {
            Impl::Io(io) => fmt::Display::fmt(io, f),
            Impl::MenuExists => {
                f.write_str("a menu already exists for the given menu target")
            }
            Impl::UnexpectedWindowType => f.write_str("unexpected window type"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &*self.0 {
            Impl::Io(io) => Some(io),
            _ => None,
        }
    }
}

impl Error {
    #[allow(unused)]
    fn last_io_error() -> Self {
        Impl::Io(std::io::Error::last_os_error()).into()
    }

    #[allow(unused)]
    fn menu_exists() -> Self {
        Impl::MenuExists.into()
    }

    #[allow(unused)]
    fn unexpected_window_type() -> Self {
        Impl::UnexpectedWindowType.into()
    }
}

impl From<Impl> for Error {
    fn from(value: Impl) -> Self {
        Self(Box::new(value))
    }
}
