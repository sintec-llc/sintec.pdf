//! Sintec.PDF: pages from one open document into another (the Pages panel's drag between
//! documents, `page_transfer` in automation). A copy is one ordinary edit of the target; a move
//! is that edit and the deletion from the source, each undone on its own document.

use super::{Arc, DocId, Edit, EditError, Session};

/// What [`Session::transfer_pages`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transferred {
    /// Pages added to the target.
    pub copied: usize,
    /// Where the first of them went (0-based).
    pub at: usize,
    /// They also left the source (a move).
    pub removed: bool,
}

impl Session {
    /// The edit that puts `pages` (0-based, in this order) of document `from`, as it is now with
    /// its unsaved changes, into document `to` before its page `at` (`None`: after its last
    /// page). Apply it to `to`. Refused when the two are the same document, no page is given, a
    /// page doesn't exist, `at` is past the end, or `from`'s security settings withhold
    /// extracting pages; the target's own permission is checked when the edit is applied.
    pub fn page_transfer_edit(&self, from: DocId, pages: &[usize], to: DocId, at: Option<usize>) -> Result<Edit, EditError> {
        if from == to {
            return Err(EditError::Invalid("the pages are already in that document: pick another one".into()));
        }
        if pages.is_empty() {
            return Err(EditError::Invalid("no pages to copy: pick at least one".into()));
        }
        let source = self.get(from).ok_or(EditError::NoDocument)?;
        let target = self.get(to).ok_or(EditError::NoDocument)?;
        let n = source.info.pages.len();
        if let Some(&p) = pages.iter().find(|&&p| p >= n) {
            return Err(EditError::Invalid(format!("page {} is past the end: {} has {n} pages", p.saturating_add(1), source.name)));
        }
        if !source.allows_page_extraction() {
            return Err(EditError::NotPermitted("copying its pages into another document"));
        }
        let end = target.info.pages.len();
        let at = at.unwrap_or(end);
        if at > end {
            return Err(EditError::Invalid(format!("position {} is past the end: {} has {end} pages", at.saturating_add(1), target.name)));
        }
        Ok(Edit::InsertPagesFrom {
            name: source.name.clone(),
            bytes: Arc::clone(&source.bytes),
            pages: Some(pages.to_vec()),
            at,
            field_conflicts: pdfcraft_organize::FieldConflicts::Rename,
        })
    }

    /// Copy `pages` (0-based) of `from` into `to` before its page `at` (`None`: at the end); with
    /// `remove`, they then leave `from` (a move). A move that would leave `from` without pages,
    /// or that `from` can't make (read only, or its security withholds page changes), is refused
    /// before anything changes. If the deletion fails anyway, the copy is undone again.
    pub fn transfer_pages(&mut self, from: DocId, pages: &[usize], to: DocId, at: Option<usize>, remove: bool) -> Result<Transferred, EditError> {
        let edit = self.page_transfer_edit(from, pages, to, at)?;
        let at = match &edit {
            Edit::InsertPagesFrom { at, .. } => *at,
            _ => 0,
        };
        let mut moving: Vec<usize> = pages.to_vec();
        moving.sort_unstable();
        moving.dedup();
        if remove {
            let source = self.get(from).ok_or(EditError::NoDocument)?;
            if moving.len() >= source.info.pages.len() {
                return Err(EditError::Invalid("moving every page would leave the document empty: copy them instead".into()));
            }
            if let Some(reason) = &source.read_only_reason {
                return Err(EditError::ReadOnly(reason.clone()));
            }
            if !source.allows_assembly() {
                return Err(EditError::NotPermitted("removing its pages"));
            }
        }
        self.apply(to, edit)?;
        if remove && let Err(e) = self.apply(from, Edit::DeletePages { pages: moving }) {
            // Best effort: the target goes back to what it was, and the error is the deletion's.
            let _ = self.undo(to);
            return Err(e);
        }
        Ok(Transferred { copied: pages.len(), at, removed: remove })
    }
}

#[cfg(test)]
mod tests {
    use super::{EditError, Session, Transferred};

    /// Two documents: `a` of three letter-size pages, `b` of two 300 × 400 pages.
    fn two() -> (Session, crate::DocId, crate::DocId) {
        let mut s = Session::new();
        let a = s.create_blank(612.0, 792.0, 3).unwrap();
        let a = s.open("a.pdf", None, a, None).unwrap();
        let b = s.create_blank(300.0, 400.0, 2).unwrap();
        let b = s.open("b.pdf", None, b, None).unwrap();
        (s, a, b)
    }

    fn widths(s: &Session, id: crate::DocId) -> Vec<f32> {
        s.get(id).unwrap().info.pages.iter().map(|p| p.width).collect()
    }

    #[test]
    fn pages_copy_into_another_document_where_asked() {
        let (mut s, a, b) = two();
        let done = s.transfer_pages(a, &[1, 2], b, Some(1), false).unwrap();
        assert_eq!(done, Transferred { copied: 2, at: 1, removed: false });
        assert_eq!(widths(&s, b), [300.0, 612.0, 612.0, 300.0]);
        assert_eq!(widths(&s, a).len(), 3, "a copy leaves the original as it was");
        // At the end by default; each side undoes on its own.
        s.transfer_pages(a, &[0], b, None, false).unwrap();
        assert_eq!(widths(&s, b), [300.0, 612.0, 612.0, 300.0, 612.0]);
        s.undo(b).unwrap();
        assert_eq!(widths(&s, b).len(), 4);
    }

    #[test]
    fn a_move_takes_the_pages_out_of_the_original_but_never_its_last_page() {
        let (mut s, a, b) = two();
        let done = s.transfer_pages(a, &[0], b, None, true).unwrap();
        assert!(done.removed);
        assert_eq!((widths(&s, a).len(), widths(&s, b).len()), (2, 3));
        assert!(s.get(a).unwrap().dirty && s.get(b).unwrap().dirty);
        // Moving every page left would empty a.pdf: refused, nothing changes.
        let e = s.transfer_pages(a, &[0, 1], b, None, true).unwrap_err();
        assert!(e.to_string().contains("empty"), "{e}");
        assert_eq!((widths(&s, a).len(), widths(&s, b).len()), (2, 3));
        // Undoing the move is two undos, one on each side.
        s.undo(a).unwrap();
        s.undo(b).unwrap();
        assert_eq!((widths(&s, a).len(), widths(&s, b).len()), (3, 2));
    }

    #[test]
    fn bad_requests_are_refused_with_what_is_allowed() {
        let (mut s, a, b) = two();
        let same = s.transfer_pages(a, &[0], a, None, false).unwrap_err();
        assert!(same.to_string().contains("another"), "{same}");
        assert!(s.transfer_pages(a, &[], b, None, false).is_err());
        let past = s.transfer_pages(a, &[3], b, None, false).unwrap_err();
        assert!(past.to_string().contains("page 4 is past the end"), "{past}");
        let gap = s.transfer_pages(a, &[0], b, Some(3), false).unwrap_err();
        assert!(gap.to_string().contains("past the end"), "{gap}");
        assert!(matches!(s.transfer_pages(a, &[0], crate::DocId(9999), None, false), Err(EditError::NoDocument)));
        assert_eq!((widths(&s, a).len(), widths(&s, b).len()), (3, 2));
    }

    /// The pages taken are the document as it is now, unsaved edits included.
    #[test]
    fn the_source_is_taken_with_its_unsaved_edits() {
        let (mut s, a, b) = two();
        s.apply(a, crate::Edit::RotatePages { pages: vec![0], degrees: 90 }).unwrap();
        s.transfer_pages(a, &[0], b, Some(0), false).unwrap();
        assert_eq!(s.get(b).unwrap().info.pages.first().map(|p| p.rotation), Some(90));
    }
}
