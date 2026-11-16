//! Reading merged areas: from the sheets a caller named to the areas each
//! sheet's part declares merged.
//!
//! A merged area's value is held in its top-left cell and the cells beside
//! it hold nothing, so a caller stepping along a row needs the areas to know
//! which cell a position's value is in. Nothing here reads a cell: the areas
//! are reported as the part declares them, each beside its anchor, and what
//! else they cover is the caller's to work out.

use std::io::{Read, Seek};

use crate::error::Result;
use crate::package::Package;
use crate::reference::Area;
use crate::relationships::Relationships;
use crate::target::resolve_sheet;
use crate::workbook::Workbook;
use crate::worksheet;

/// One sheet a `merged` operand named, and the areas its part declares
/// merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedReport {
    /// The sheet, in the package's own spelling.
    pub sheet: String,
    /// Each area, in the order the part declares them.
    pub areas: Vec<Area>,
}

/// Read the merged areas of each sheet `sheets` names, in the order the
/// sheets were named.
///
/// Every sheet is resolved before any part is read, as `cells` resolves
/// them, so a sheet the package lacks fails the read before a part is
/// opened. A sheet named twice is answered twice; its part is parsed once.
/// One area that is not a range fails the whole read, naming the sheet.
pub fn read<R: Read + Seek>(
    package: &mut Package<R>,
    workbook: &Workbook,
    sheets: &[String],
) -> Result<Vec<MergedReport>> {
    let rels = Relationships::read(package, workbook.part())?;
    let resolved: Vec<(String, String)> = sheets
        .iter()
        .map(|sheet| resolve_sheet(package, &rels, workbook, sheet))
        .collect::<Result<_>>()?;

    let mut held: Vec<(String, Vec<Area>)> = Vec::new();
    for (sheet, part) in &resolved {
        if held.iter().any(|(read, _)| read == part) {
            continue;
        }
        let xml = package.read_part_text(part)?;
        let document = worksheet::parsed(xml).map_err(|err| err.within(part))?;
        let areas = worksheet::merged_areas(&document, sheet).map_err(|err| err.within(part))?;
        held.push((part.clone(), areas));
    }

    Ok(resolved
        .into_iter()
        .map(|(sheet, part)| MergedReport {
            areas: held
                .iter()
                .find(|(read, _)| *read == part)
                .map(|(_, areas)| areas.clone())
                .unwrap_or_default(),
            sheet,
        })
        .collect())
}
