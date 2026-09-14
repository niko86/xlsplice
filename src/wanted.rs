//! What a batch wants written, and who asked for it.
//!
//! An operation answers with the edits it wants made to each part
//! ([`Asked`]), and several operations may land on one part. What that part is
//! to become is therefore a question about the batch rather than about any one
//! operation, and this is where it is held: the edits merged part by part,
//! which operation asked for which of them, and which operations have turned
//! out to change a byte.
//!
//! Three questions have no per-operation answer at all — whether an emptied
//! calc chain still belongs in the package, how many properties are being
//! added and which identifiers they take, and how many cells a row being put
//! in holds. ADR-0004 calls them settled once, for the part, over one
//! snapshot. A [`Settlement`] is one of those answers: it is handed the open
//! package and this, and may read what the batch already wants, merge more
//! into it, and credit the operations it answered for. Nothing else about a
//! batch is reachable through it, which is what keeps a settlement a question
//! about parts rather than a fourth place operations are interpreted.
//!
//! The settlements are independent of one another — see `SETTLEMENTS` in
//! [`crate::batch`], which says so and is held to it by a test — so this says
//! nothing about the order they run in.
//!
//! [`Wanted::into_applied`] is the end of it: every part taken once, in path
//! order, read once, spliced once with all of its splices and written once.

use std::collections::BTreeMap;
use std::io::{Read, Seek};

use crate::batch::{Asked, Opened, Operation, PartEdit, Parts};
use crate::error::{Error, Result};
use crate::package::Content;
use crate::splice;

/// One of ADR-0004's batch-level answers: the open package and what the batch
/// wants of it so far, in, and whatever more it wants, out.
pub type Settlement<R> = fn(&mut Opened<R>, &mut Wanted) -> Result<()>;

/// What a batch wants written, and who asked for it.
pub struct Wanted<'a> {
    operations: &'a [Operation],
    asked: &'a [Asked],
    /// What each part is to have done to it, by path. What the operations
    /// asked for, and then what the settlements added.
    edits: BTreeMap<String, PartEdit>,
    /// Which operations asked for which of a part's edits, by path. An
    /// operation may be in several parts' lists.
    askers: BTreeMap<String, Vec<(usize, &'a PartEdit)>>,
    /// Whether each operation changed a byte, by its place in the batch.
    changed: Vec<bool>,
}

impl<'a> Wanted<'a> {
    /// What the operations asked for, merged part by part.
    ///
    /// Two operations may both be done with one part, and what they come to
    /// together is worked out here, before any settlement sees it: a
    /// settlement asking what the batch wants of a part is asking about the
    /// whole batch.
    pub fn of(operations: &'a [Operation], asked: &'a [Asked]) -> Result<Self> {
        let askers = askers(asked);
        let mut edits = BTreeMap::new();
        for (part, asking) in &askers {
            edits.insert(part.clone(), merged(part, asking)?);
        }
        Ok(Wanted {
            operations,
            asked,
            edits,
            askers,
            changed: vec![false; asked.len()],
        })
    }

    /// The operations, for a settlement that is about what was asked for
    /// rather than about what it came to.
    pub fn operations(&self) -> &'a [Operation] {
        self.operations
    }

    /// What every operation answered with.
    pub fn asked(&self) -> &'a [Asked] {
        self.asked
    }

    /// What the batch wants of `part`, or nothing where it wants nothing.
    pub fn of_part(&self, part: &str) -> Option<&PartEdit> {
        self.edits.get(part)
    }

    /// Fold `edit` into what the batch already wants of `part`.
    ///
    /// Two edits that disagree about what is being done to one part are a
    /// fault in how they were worked out rather than something the package
    /// can be wrong about, so the failure says which batch-level question was
    /// being answered when they met.
    pub fn merge(&mut self, part: String, edit: PartEdit, settling: &str) -> Result<()> {
        let merged = match self.edits.get(&part) {
            None => edit,
            Some(already) => already.and(&edit).ok_or_else(|| {
                Error::internal(format!(
                    "part '{part}' is being edited two ways at once, settling {settling}"
                ))
            })?,
        };
        self.edits.insert(part, merged);
        Ok(())
    }

    /// The batch wants `part` gone, whatever it wanted done to it before.
    ///
    /// This replaces rather than folds, which is the one place that is the
    /// right answer: the splices anyone asked for in a part that is being
    /// removed land in text that will not be there. Only a settlement can say
    /// so, because only a settlement sees the part as the whole batch leaves
    /// it — an operation asking for a splice cannot know that another one
    /// took the last entry out.
    pub fn withdraw(&mut self, part: String) {
        self.edits.insert(part, PartEdit::Remove);
    }

    /// The same, for every part a declaration pass answered with.
    pub fn merge_all(
        &mut self,
        declarations: Vec<(String, Vec<crate::splice::Splice>)>,
        settling: &str,
    ) -> Result<()> {
        for (part, splices) in declarations {
            self.merge(part, PartEdit::Splice(splices), settling)?;
        }
        Ok(())
    }

    /// The operation at `index` changed a byte.
    ///
    /// An operation may edit several parts and the parts are taken one at a
    /// time, so this is added to what its other parts said rather than put in
    /// place of it: an operation changed something if any one of its edits
    /// did.
    pub fn credit(&mut self, index: usize) {
        self.changed[index] = true;
    }

    /// Which operations have turned out to change a byte, for a test asking
    /// what a settlement credited.
    #[cfg(test)]
    pub fn credited(&self) -> &[bool] {
        &self.changed
    }

    /// Every operation that asked for `part` changed something. A part
    /// created or removed is a change by all of them, because the part was
    /// not there, or was, before any of them asked.
    fn credit_all_of(&mut self, part: &str) {
        for (index, _) in self.askers.get(part).into_iter().flatten() {
            self.changed[*index] = true;
        }
    }

    /// Each operation that asked for `part` changed something if one of the
    /// splices it asked for lands on a different byte than the one there.
    fn credit_the_splices_that_land(&mut self, part: &str, xml: &str) {
        for (index, edit) in self.askers.get(part).into_iter().flatten() {
            self.changed[*index] |= edit.splices().iter().any(|splice| splice.changes(xml));
        }
    }

    /// Work what the batch wants out into what each part is to become.
    ///
    /// A part is dealt with once, however many operations landed on it: read
    /// once, spliced once with all of their splices, and written once. The
    /// parts are taken in path order, which is the order the report lists
    /// them in.
    ///
    /// A part whose splices leave it saying what it already said is not a
    /// changed part: it carries no content, so a batch that would change
    /// nothing writes nothing (ADR-0003).
    pub fn into_applied<R: Read + Seek>(mut self, opened: &mut Opened<R>) -> Result<Applied> {
        let mut content = BTreeMap::new();
        let mut parts = Parts::default();
        for (part, edit) in std::mem::take(&mut self.edits) {
            match edit {
                PartEdit::Splice(all) => {
                    let (spliced, landed) = opened.read_part(&part, |xml| {
                        let spliced = splice::apply(xml, &all)?;
                        let landed = spliced != xml;
                        self.credit_the_splices_that_land(&part, xml);
                        Ok((spliced, landed))
                    })?;
                    if landed {
                        parts.changed.push(part.clone());
                        content.insert(part, Content::Bytes(spliced.into_bytes()));
                    }
                }
                PartEdit::Create(bytes) => {
                    if opened.has_part(&part) {
                        return Err(Error::internal(format!(
                            "part '{part}' is already in the package, so it cannot be created"
                        )));
                    }
                    self.credit_all_of(&part);
                    parts.added.push(part.clone());
                    content.insert(part, Content::Bytes(bytes));
                }
                PartEdit::Remove => {
                    if !opened.has_part(&part) {
                        return Err(Error::internal(format!(
                            "part '{part}' is not in the package, so it cannot be removed"
                        )));
                    }
                    self.credit_all_of(&part);
                    parts.removed.push(part.clone());
                    content.insert(part, Content::Gone);
                }
            }
        }
        Ok(Applied {
            content,
            changed: self.changed,
            parts,
        })
    }
}

/// What a batch's edits came to, before anything is written.
pub struct Applied {
    /// What each part the batch touched is to become.
    pub content: BTreeMap<String, Content>,
    /// Whether each operation changed a byte, by its place in the batch.
    pub changed: Vec<bool>,
    /// What to report about the parts.
    pub parts: Parts,
}

/// Every operation's edits, gathered by the part they land in, in path order.
fn askers(asked: &[Asked]) -> BTreeMap<String, Vec<(usize, &PartEdit)>> {
    let mut askers: BTreeMap<String, Vec<(usize, &PartEdit)>> = BTreeMap::new();
    for (index, edits) in asked.iter().enumerate() {
        for (part, edit) in &edits.parts {
            askers.entry(part.clone()).or_default().push((index, edit));
        }
    }
    askers
}

/// What the edits on one part come to together.
///
/// The edits are folded into one, and two that disagree about what is being
/// done to the part are two operations asking different things of it: a fault
/// in whatever built the batch rather than something the package can be wrong
/// about, so the failure names both of them.
fn merged(part: &str, edits: &[(usize, &PartEdit)]) -> Result<PartEdit> {
    let Some(((first, edit), rest)) = edits.split_first() else {
        return Err(Error::internal(format!(
            "part '{part}' is named by no edit"
        )));
    };
    let mut merged = (*edit).clone();
    for (other, edit) in rest {
        merged = merged.and(edit).ok_or_else(|| {
            Error::internal(format!(
                "the operations at index {first} and {other} ask different things of part \
                 '{part}': a part is spliced, created or removed, not two of the three"
            ))
        })?;
    }
    Ok(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::splice::Splice;

    fn splice() -> Splice {
        Splice::new(0..1, "x")
    }

    /// One operation, answering with an edit for each of `parts`.
    fn asking(parts: &[(&str, PartEdit)]) -> Asked {
        Asked {
            at: None,
            parts: parts
                .iter()
                .map(|(part, edit)| ((*part).to_owned(), edit.clone()))
                .collect(),
            into_a_new_row: None,
        }
    }

    /// What the operations asked for is what the batch starts out wanting,
    /// part by part, before a settlement has said anything.
    #[test]
    fn a_batch_starts_out_wanting_what_its_operations_asked_for() {
        let asked = [
            asking(&[("a.xml", PartEdit::Splice(vec![splice()]))]),
            asking(&[("b.xml", PartEdit::Remove)]),
        ];

        let wanted = Wanted::of(&[], &asked).expect("two operations, two parts");

        assert_eq!(
            wanted.of_part("a.xml"),
            Some(&PartEdit::Splice(vec![splice()]))
        );
        assert_eq!(wanted.of_part("b.xml"), Some(&PartEdit::Remove));
        assert_eq!(wanted.of_part("c.xml"), None, "a part nobody named");
        assert_eq!(wanted.changed, [false, false], "nothing has been applied");
    }

    /// A settlement folds what it wants into what is already there, and the
    /// failure names the question being answered when two disagreed.
    #[test]
    fn what_a_settlement_merges_folds_into_what_the_operations_asked() {
        let asked = [asking(&[("a.xml", PartEdit::Splice(vec![splice()]))])];
        let mut wanted = Wanted::of(&[], &asked).expect("one operation");

        wanted
            .merge(
                "a.xml".to_owned(),
                PartEdit::Splice(vec![Splice::new(4..5, "y")]),
                "a question",
            )
            .expect("two splice lists are one list");
        let err = wanted
            .merge("a.xml".to_owned(), PartEdit::Remove, "another question")
            .expect_err("a part is not spliced and removed at once");

        assert_eq!(
            wanted.of_part("a.xml"),
            Some(&PartEdit::Splice(vec![splice(), Splice::new(4..5, "y")]))
        );
        assert_eq!(err.code(), ErrorCode::Internal);
        assert!(
            err.message().contains("another question"),
            "the failure names what was being settled: {}",
            err.message()
        );
    }

    /// Two operations landing on one part are one splice list against one
    /// text; whether two of those ranges collide is `splice::apply`'s to say,
    /// not this.
    #[test]
    fn the_splices_of_every_operation_on_one_part_are_merged_into_one_list() {
        let one = PartEdit::Splice(vec![splice()]);
        let two = PartEdit::Splice(vec![Splice::new(4..5, "y"), Splice::new(9..9, "z")]);

        let merged = merged("sheet1.xml", &[(0, &one), (1, &two)]).expect("splices merge");

        assert_eq!(
            merged,
            PartEdit::Splice(vec![
                splice(),
                Splice::new(4..5, "y"),
                Splice::new(9..9, "z")
            ])
        );
    }

    /// Two operations may both be done with one part, and a part is removed
    /// once however many of them said so.
    #[test]
    fn two_operations_removing_one_part_remove_it_once() {
        let merged = merged(
            "xl/calcChain.xml",
            &[(0, &PartEdit::Remove), (1, &PartEdit::Remove)],
        )
        .expect("two removals are one removal");

        assert_eq!(merged, PartEdit::Remove);
    }

    /// Two operations creating one part agree only if they agree about what
    /// is in it.
    #[test]
    fn two_operations_creating_one_part_must_carry_the_same_bytes() {
        let one = PartEdit::Create(b"<properties/>".to_vec());
        let same = PartEdit::Create(b"<properties/>".to_vec());
        let other = PartEdit::Create(b"<properties count=\"1\"/>".to_vec());

        assert_eq!(
            merged("docProps/custom.xml", &[(0, &one), (1, &same)]).expect("the same bytes"),
            one
        );
        let err = merged("docProps/custom.xml", &[(0, &one), (1, &other)])
            .expect_err("different bytes are two answers to one question");
        assert_eq!(err.code(), ErrorCode::Internal);
    }

    /// A part is spliced, created or removed, not two of the three. Whatever
    /// built such a batch is at fault, so the failure names both operations.
    #[test]
    fn edits_of_different_kinds_on_one_part_are_refused_naming_both_operations() {
        let splicing = PartEdit::Splice(vec![splice()]);

        let err = merged("sheet1.xml", &[(2, &splicing), (5, &PartEdit::Remove)])
            .expect_err("a part is not spliced and removed at once");

        assert_eq!(err.code(), ErrorCode::Internal);
        assert!(err.message().contains("index 2 and 5"), "{}", err.message());
        assert!(err.message().contains("sheet1.xml"), "{}", err.message());
    }

    /// An operation may edit several parts, and the parts are taken one at a
    /// time. What one of them says about an operation is added to what the
    /// others said, so a part an operation changed nothing in cannot take
    /// back a part it did change. No operation edits two parts yet; #10 and
    /// #12 are the first that will.
    #[test]
    fn an_operation_that_changed_any_of_its_parts_changed_something() {
        let changes = PartEdit::Splice(vec![Splice::new(0..1, "y")]);
        let does_not = PartEdit::Splice(vec![Splice::new(0..1, "x")]);
        let asked = [asking(&[
            ("changed.xml", changes),
            ("unchanged.xml", does_not.clone()),
            ("gone.xml", PartEdit::Remove),
        ])];
        let mut wanted = Wanted::of(&[], &asked).expect("one operation, three parts");

        wanted.credit_the_splices_that_land("changed.xml", "xxx");
        assert!(
            wanted.changed[0],
            "the splice put a byte there that was not"
        );
        wanted.credit_the_splices_that_land("unchanged.xml", "xxx");
        assert!(
            wanted.changed[0],
            "a later part it changed nothing in does not take that back"
        );

        let mut removing = Wanted::of(&[], &asked).expect("one operation, three parts");
        removing.credit_all_of("gone.xml");
        removing.credit_the_splices_that_land("unchanged.xml", "xxx");
        assert!(
            removing.changed[0],
            "and neither does one after a part removed"
        );
    }
}
