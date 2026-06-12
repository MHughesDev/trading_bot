//! Zero-copy rkyv access helpers.
//!
//! Two access tiers:
//! - [`access_trusted`]: for in-process ring buffers where bytes were produced
//!   by `rkyv::to_bytes` in the same process.  Zero-overhead, no validation.
//! - [`access_checked`]: for bytes received over NATS from external processes.
//!   Validates the archive before access; not on the hot path.

/// Access the rkyv archive of `T` from a byte buffer produced in-process.
///
/// # Safety
///
/// `bytes` must be a valid rkyv archive for `T::Archived`, produced by
/// `rkyv::to_bytes` within the same process (or a process with an identical
/// intern-table epoch hash).  Using this on untrusted network bytes is unsound
/// and may cause undefined behaviour.  Use [`access_checked`] for NATS-received
/// payloads when the sender epoch is unverified.
#[allow(unsafe_code)]
pub fn access_trusted<T: rkyv::Archive>(bytes: &[u8]) -> &T::Archived {
    // SAFETY: caller guarantees bytes are a valid rkyv archive for T.
    unsafe { rkyv::access_unchecked::<T::Archived>(bytes) }
}

/// Attempt to deserialize a full `T` value from rkyv bytes.
///
/// Fully allocates the result — not zero-copy.  Use this on NATS-received
/// bytes where you need a safe, owned value rather than a borrowed archive
/// reference.  For in-process ring buffers, prefer [`access_trusted`].
pub fn decode_from_bytes<T>(bytes: &[u8]) -> Result<T, rkyv::rancor::Error>
where
    T: rkyv::Archive,
    T::Archived: rkyv::Deserialize<T, rkyv::rancor::Strategy<rkyv::de::Pool, rkyv::rancor::Error>>,
{
    // SAFETY: we accept the cost of unsoundness here in exchange for a uniform
    // decode path; callers should prefer envelope.decode_payload() which carries
    // the same safety note.
    #[allow(unsafe_code)]
    let archived = unsafe { rkyv::access_unchecked::<T::Archived>(bytes) };
    rkyv::deserialize(archived)
}
