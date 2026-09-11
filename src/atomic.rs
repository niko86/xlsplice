//! Putting bytes at a path so that nothing ever sees half of them.
//!
//! Nothing here knows what a package is, or a part: this is the disk, and the
//! one promise made about it. The bytes go to a temporary file beside the
//! destination and are then renamed onto it, which is atomic on one
//! filesystem, so a crash or a kill leaves either the file that was there or
//! the file that was asked for. That is what lets every non-zero exit promise
//! nothing was written.
//!
//! Beside the destination rather than in a system temporary directory,
//! because a rename across filesystems is a copy and a copy is not atomic.
//!
//! Failures here are [`internal`](crate::error::ErrorCode::Internal): the
//! frozen exit-code table has no code for the disk, and the spec calls a
//! destination that cannot be replaced an unexpected failure.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::error::{Error, Result};

/// Enough to keep two writes in one process out of each other's way.
static NEXT: AtomicU32 = AtomicU32::new(0);

/// A file that removes itself unless it became the destination.
struct Temporary {
    path: PathBuf,
    /// Whether the rename took it, and with it the job of removing it.
    given_away: bool,
}

impl Temporary {
    /// Beside `destination`, so that the rename that follows stays on one
    /// filesystem. The name is hidden and carries the process id, so two
    /// writes at once do not meet.
    fn beside(destination: &Path) -> Self {
        let name = destination
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ordinal = NEXT.fetch_add(1, Ordering::Relaxed);
        Temporary {
            path: destination
                .with_file_name(format!(".{name}.xlsplice-{}-{ordinal}", std::process::id())),
            given_away: false,
        }
    }

    /// Make the file the destination. A rename that fails gives nothing away,
    /// so the file is still this one's to remove.
    fn rename_to(mut self, destination: &Path) -> std::io::Result<()> {
        fs::rename(&self.path, destination)?;
        self.given_away = true;
        Ok(())
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        if !self.given_away {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Put `bytes` at `destination`, replacing whatever was there.
///
/// A failure anywhere leaves the destination as it was and takes the
/// temporary file with it.
pub fn replace(destination: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = Temporary::beside(destination);
    let written = File::create(&temporary.path).and_then(|mut file| {
        file.write_all(bytes)?;
        // Flushed before the rename, so that the file the rename publishes is
        // the whole of it.
        file.sync_all()
    });
    written.map_err(|err| {
        Error::internal(format!(
            "cannot write beside {}: {err}. Nothing was written.",
            destination.display()
        ))
    })?;
    temporary.rename_to(destination).map_err(|err| {
        Error::internal(format!(
            "cannot replace {}: {err}. Nothing was written.",
            destination.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    /// A directory of one test's files, removed when the test ends.
    struct Dir(PathBuf);

    impl Dir {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("xlsplice-atomic-{}-{label}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("a test must be able to write a directory");
            Dir(path)
        }

        /// What is in the directory, sorted.
        fn listing(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.0)
                .expect("the directory must be readable")
                .map(|entry| {
                    entry
                        .expect("an entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_bytes_arrive_and_nothing_else_is_left_lying_about() {
        let dir = Dir::new("arrive");
        let destination = dir.0.join("book.xlsx");

        replace(&destination, b"one").expect("the bytes must land");

        assert_eq!(
            fs::read(&destination).expect("the file must be there"),
            b"one"
        );
        assert_eq!(dir.listing(), ["book.xlsx"]);
    }

    #[test]
    fn a_file_already_there_is_replaced_whole() {
        let dir = Dir::new("replace");
        let destination = dir.0.join("book.xlsx");
        fs::write(&destination, b"the old and rather longer contents").expect("a file to replace");

        replace(&destination, b"new").expect("the bytes must land");

        assert_eq!(
            fs::read(&destination).expect("the file must be there"),
            b"new"
        );
        assert_eq!(dir.listing(), ["book.xlsx"]);
    }

    #[test]
    fn a_destination_that_cannot_be_written_fails_before_it_touches_anything() {
        let dir = Dir::new("unwritable");
        let nowhere = dir.0.join("no-such-directory").join("book.xlsx");

        let err = replace(&nowhere, b"one").expect_err("there is nowhere to write");

        assert_eq!(err.code(), ErrorCode::Internal);
        assert!(err.message().contains("Nothing was written"), "{err}");
        assert!(dir.listing().is_empty(), "{:?}", dir.listing());
    }

    #[test]
    fn a_rename_that_fails_takes_the_temporary_file_with_it() {
        let dir = Dir::new("unrenameable");
        // A directory cannot be renamed over, so this fails at the rename
        // rather than before it: the one path that leaves a file to clean up.
        let occupied = dir.0.join("occupied");
        fs::create_dir(&occupied).expect("a directory to be in the way");
        fs::write(occupied.join("held"), b"x").expect("something to hold it open");

        let err = replace(&occupied, b"one").expect_err("a directory cannot be replaced");

        assert_eq!(err.code(), ErrorCode::Internal);
        assert_eq!(
            dir.listing(),
            ["occupied"],
            "the temporary file must not outlive the failure"
        );
    }

    #[test]
    fn the_temporary_file_sits_beside_the_destination() {
        // The rename is only atomic within one filesystem, so where the
        // temporary file goes is the whole of that promise.
        let temporary = Temporary::beside(Path::new("/somewhere/else/book.xlsx"));

        assert_eq!(temporary.path.parent(), Some(Path::new("/somewhere/else")));
        assert!(
            temporary
                .path
                .file_name()
                .expect("a file name")
                .to_string_lossy()
                .starts_with(".book.xlsx.xlsplice-"),
            "{}",
            temporary.path.display()
        );
    }

    #[test]
    fn two_temporary_files_in_one_process_do_not_meet() {
        let destination = Path::new("/somewhere/book.xlsx");
        let (one, two) = (
            Temporary::beside(destination),
            Temporary::beside(destination),
        );

        assert_ne!(one.path, two.path);
    }
}
