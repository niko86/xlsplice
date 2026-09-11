//! The one typed error the library returns, and the frozen exit-code table.
//!
//! Every failure in `xlsplice` is an [`Error`] carrying an [`ErrorCode`]. The
//! code is what a caller reads: it appears verbatim as `error.code` in the
//! JSON envelope and it decides the process exit code. The table is frozen:
//!
//! | Exit | Code         | Meaning                                       |
//! |------|--------------|-----------------------------------------------|
//! | 0    | —            | Success.                                      |
//! | 1    | `internal`   | Unexpected failure, including a caught panic. |
//! | 2    | `usage`      | The command line was wrong.                   |
//! | 3    | `not_found`  | A sheet, name, cell or part was not found.    |
//! | 4    | `refused`    | A guard said no.                              |
//! | 5    | `unreadable` | Not a package, or the package cannot be read. |
//!
//! Any non-zero code guarantees that no package was written. Changes are
//! additive: codes may be added, never removed or renumbered, so consumers
//! must tolerate a code they do not recognise.

use std::fmt;

/// The exit code of a successful command.
pub const EXIT_SUCCESS: u8 = 0;

/// The stable, snake-case error code carried by every [`Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// An unexpected failure, including a caught panic. Exit 1.
    Internal,
    /// The command line was wrong. Exit 2.
    Usage,
    /// A sheet, defined name, cell or part was not found. Exit 3.
    NotFound,
    /// A guard refused the operation. Exit 4.
    Refused,
    /// Not a package, or the package cannot be read. Exit 5.
    Unreadable,
}

impl ErrorCode {
    /// Every code in the frozen table, in exit-code order.
    pub const ALL: [ErrorCode; 5] = [
        ErrorCode::Internal,
        ErrorCode::Usage,
        ErrorCode::NotFound,
        ErrorCode::Refused,
        ErrorCode::Unreadable,
    ];

    /// The code as it appears in `error.code` in the JSON envelope.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Internal => "internal",
            ErrorCode::Usage => "usage",
            ErrorCode::NotFound => "not_found",
            ErrorCode::Refused => "refused",
            ErrorCode::Unreadable => "unreadable",
        }
    }

    /// The process exit code this error code maps to.
    pub fn exit_code(self) -> u8 {
        match self {
            ErrorCode::Internal => 1,
            ErrorCode::Usage => 2,
            ErrorCode::NotFound => 3,
            ErrorCode::Refused => 4,
            ErrorCode::Unreadable => 5,
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A failure, with the code that decides how it is reported and what the
/// process exits with, and a message that names the fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    code: ErrorCode,
    message: String,
}

impl Error {
    /// Build an error with an explicit code.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Error {
            code,
            message: message.into(),
        }
    }

    /// An unexpected failure, including a caught panic. Exit 1.
    pub fn internal(message: impl Into<String>) -> Self {
        Error::new(ErrorCode::Internal, message)
    }

    /// The command line was wrong. Exit 2.
    pub fn usage(message: impl Into<String>) -> Self {
        Error::new(ErrorCode::Usage, message)
    }

    /// A sheet, defined name, cell or part was not found. Exit 3.
    pub fn not_found(message: impl Into<String>) -> Self {
        Error::new(ErrorCode::NotFound, message)
    }

    /// A guard refused the operation. Exit 4.
    pub fn refused(message: impl Into<String>) -> Self {
        Error::new(ErrorCode::Refused, message)
    }

    /// Not a package, or the package cannot be read. Exit 5.
    pub fn unreadable(message: impl Into<String>) -> Self {
        Error::new(ErrorCode::Unreadable, message)
    }

    /// The stable code, as it appears in the envelope.
    pub fn code(&self) -> ErrorCode {
        self.code
    }

    /// The human-readable message, which should name the fix.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The process exit code for this error.
    pub fn exit_code(&self) -> u8 {
        self.code.exit_code()
    }

    /// The same failure, said again with where it happened in front of it.
    /// The code is kept, because where a failure happened does not change
    /// what it was.
    pub fn within(self, place: impl fmt::Display) -> Self {
        Error::new(self.code, format!("{place}: {self}"))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// The result of any fallible library operation.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is frozen. These literals are transcribed from the spec, not
    /// derived from the code, so a renumbering here fails the test.
    const TABLE: [(ErrorCode, &str, u8); 5] = [
        (ErrorCode::Internal, "internal", 1),
        (ErrorCode::Usage, "usage", 2),
        (ErrorCode::NotFound, "not_found", 3),
        (ErrorCode::Refused, "refused", 4),
        (ErrorCode::Unreadable, "unreadable", 5),
    ];

    #[test]
    fn every_code_has_its_frozen_name_and_exit_code() {
        for (code, name, exit) in TABLE {
            assert_eq!(code.as_str(), name);
            assert_eq!(code.exit_code(), exit);
        }
    }

    #[test]
    fn success_is_exit_zero_and_no_error_code_claims_it() {
        assert_eq!(EXIT_SUCCESS, 0);
        assert!(ErrorCode::ALL.iter().all(|c| c.exit_code() != EXIT_SUCCESS));
    }

    #[test]
    fn the_table_lists_every_code_exactly_once() {
        assert_eq!(ErrorCode::ALL.len(), TABLE.len());
        for (code, _, _) in TABLE {
            assert_eq!(ErrorCode::ALL.iter().filter(|c| **c == code).count(), 1);
        }
    }

    #[test]
    fn saying_where_a_failure_happened_keeps_the_code_it_had() {
        let err = Error::unreadable("not valid XML").within("xl/worksheets/sheet1.xml");

        assert_eq!(err.code(), ErrorCode::Unreadable);
        assert_eq!(err.message(), "xl/worksheets/sheet1.xml: not valid XML");
    }

    #[test]
    fn an_error_carries_its_code_and_message() {
        let err = Error::not_found("no sheet named 'Inputs'; the package has: Sheet1");
        assert_eq!(err.code(), ErrorCode::NotFound);
        assert_eq!(err.exit_code(), 3);
        assert_eq!(
            err.message(),
            "no sheet named 'Inputs'; the package has: Sheet1"
        );
        assert_eq!(
            err.to_string(),
            "no sheet named 'Inputs'; the package has: Sheet1"
        );
    }
}
