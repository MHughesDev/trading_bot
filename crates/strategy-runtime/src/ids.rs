//! Compact numeric ID newtypes for strategy-runtime graph structures.

/// Opaque u32 handle for a strategy graph node.
///
/// Assigned sequentially at compile time from the node `Vec` index.
/// Replaces `String` node IDs in hot-path `HashMap` keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u32);
