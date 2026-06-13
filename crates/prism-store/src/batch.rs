//! A higher-level write transaction: collect object records and typed index
//! entries, then commit them in a single atomic manifest swap.
//!
//! A batch is the unit of durability (architecture doc, *Crash Safety*): the
//! `.pseg` of records and the `.pidx` that indexes them are staged together and
//! published by one manifest swap, so the index never references an unpublished
//! record.

use crate::RawRecord;
use prism_model::StringId;
use std::collections::HashMap;

/// A pending index put or delete, recorded with its key. The `logical_ts` is
/// assigned by the store at commit time from the recovered high-water mark.
#[derive(Clone, Debug)]
pub(crate) struct IndexOp {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
    pub tombstone: bool,
}

/// A staged set of object records and index entries to commit atomically.
///
/// Build one with [`crate::Store::batch`], add records/index entries, then pass it
/// to [`crate::Store::commit_batch`].
#[derive(Default)]
pub struct WriteBatch {
    pub(crate) records: Vec<RawRecord>,
    pub(crate) index_ops: Vec<IndexOp>,
    /// Within-batch intern dedup: text SHA-256 -> the id assigned earlier in this
    /// same uncommitted batch, so repeated interns of one string don't re-allocate.
    pub(crate) pending_strings: HashMap<[u8; 32], StringId>,
}

impl WriteBatch {
    /// A new empty batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an object record to the batch's segment.
    pub fn put_record(&mut self, record: RawRecord) -> &mut Self {
        self.records.push(record);
        self
    }

    /// Add a typed index entry (`key -> value`). Last write at commit wins by key
    /// within a single batch.
    pub fn put_index(&mut self, key: Vec<u8>, value: Vec<u8>) -> &mut Self {
        self.index_ops.push(IndexOp {
            key,
            value,
            tombstone: false,
        });
        self
    }

    /// Tombstone a typed index key.
    pub fn delete_index(&mut self, key: Vec<u8>) -> &mut Self {
        self.index_ops.push(IndexOp {
            key,
            value: Vec::new(),
            tombstone: true,
        });
        self
    }

    /// True if the batch has neither records nor index entries.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty() && self.index_ops.is_empty()
    }

    /// Number of object records staged.
    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    /// Number of index operations staged.
    pub fn index_op_count(&self) -> usize {
        self.index_ops.len()
    }
}
