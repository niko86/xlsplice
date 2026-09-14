//! What the suites are built out of, in four clusters and one name each.
//!
//! Every suite declares `mod support;` and imports the clusters it draws on,
//! which is how a suite says what kind of test it is: [`workspace`] to put a
//! package somewhere, [`container`] to read one back and hold it to the
//! guarantee, [`library`] to call a verb in process, [`binary`] to spawn the
//! tool and read its streams.
//!
//! They were one module until #36. The flat namespace had one name meaning two
//! things — `CONTENT_TYPES` was the text of the part and `CONTENT_TYPES`
//! its path — which is why ten suites had taken to declaring their own
//! `SHEET1` rather than importing one. A name now belongs to the cluster it
//! means something in.

/// The corpus, and the fixed operation set every package in it is put
/// through.
pub mod corpus;
/// A real Excel, for the one question the tool must not answer about itself.
pub mod oracle;

/// Spawning the tool: argv in, two streams and an exit code out.
pub mod binary;
/// Reading a package back, and the comparison that holds the guarantee.
pub mod container;
/// Calling a verb in process, and the operations a batch is made of.
pub mod library;
/// Somewhere to put a package, and the packages a test builds itself.
pub mod workspace;
