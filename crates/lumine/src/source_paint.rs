//! Native host metadata, deliberately separate from authored style/props.
//! A capability records a host decision; it does not independently qualify a
//! browser backend. Trusted source/build lookup must precede installing it.
use std::num::NonZeroU64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourcePaintPolicy {
    SkiaSoftwareSrgb8OpaqueBackdrop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourcePaintCapability {
    pub owner: NonZeroU64,
    pub contract_sha256: [u8; 32],
    pub policy: SourcePaintPolicy,
}

/// An explicit source-root boundary. PreserveExisting prevents an imported
/// native subtree from inheriting its parent's source-specific policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourcePaintScope {
    PreserveExisting,
    Capability(SourcePaintCapability),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SourcePaintState {
    #[default]
    Absent,
    Active(SourcePaintCapability),
    Blocked,
}
impl SourcePaintState {
    pub(crate) fn descend(self, scope: Option<SourcePaintScope>) -> Self {
        match (self, scope) {
            // A conflict or explicit preserve boundary is sticky inside that
            // subtree: a deeper marker cannot restore a conflicting policy.
            (Self::Blocked, _) | (_, Some(SourcePaintScope::PreserveExisting)) => Self::Blocked,
            (state, None) => state,
            (Self::Absent, Some(SourcePaintScope::Capability(cap))) => Self::Active(cap),
            (Self::Active(parent), Some(SourcePaintScope::Capability(cap))) if parent == cap => {
                Self::Active(cap)
            }
            (Self::Active(_), Some(SourcePaintScope::Capability(_))) => Self::Blocked,
        }
    }
    pub(crate) fn capability(self) -> Option<SourcePaintCapability> {
        match self {
            Self::Active(cap) => Some(cap),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cap(owner: u64) -> SourcePaintCapability {
        SourcePaintCapability {
            owner: NonZeroU64::new(owner).unwrap(),
            contract_sha256: [owner as u8; 32],
            policy: SourcePaintPolicy::SkiaSoftwareSrgb8OpaqueBackdrop,
        }
    }
    #[test]
    fn absent_and_identical_nested_roots_preserve_ownership() {
        assert_eq!(SourcePaintState::default().descend(None).capability(), None);
        let scope = Some(SourcePaintScope::Capability(cap(1)));
        let state = SourcePaintState::default().descend(scope);
        assert_eq!(
            state.descend(None).descend(scope).capability(),
            Some(cap(1))
        );
        assert_eq!(SourcePaintState::default().capability(), None);
    }
    #[test]
    fn conflicting_owner_contract_and_preserve_boundaries_fail_closed() {
        let state = SourcePaintState::Active(cap(1));
        let mut changed_contract = cap(1);
        changed_contract.contract_sha256 = [3; 32];
        for scope in [
            SourcePaintScope::Capability(cap(2)),
            SourcePaintScope::Capability(changed_contract),
            SourcePaintScope::PreserveExisting,
        ] {
            let blocked = state.descend(Some(scope));
            assert_eq!(blocked, SourcePaintState::Blocked);
            assert_eq!(
                blocked
                    .descend(Some(SourcePaintScope::Capability(cap(1))))
                    .capability(),
                None
            );
        }
    }
}
