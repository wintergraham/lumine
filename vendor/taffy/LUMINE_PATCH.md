# Lumine patch to Taffy 0.9.0

The source is copied from the installed crates.io `taffy-0.9.0` package.
Its `.cargo_vcs_info.json` identifies upstream commit
`599b1b7d7410d97e13bafc4c01fbe6d7a9cf060a`.
The cached crate archive has SHA-256
`a13e5d13f79d558b5d353a98072ca8ca0e99da429467804de959aa8c83c9a004`.

All packaged Rust sources, examples, both manifests, README and VCS metadata
are retained. The registry installation marker and packaged development
`Cargo.lock` are omitted. The consuming workspace lockfile owns resolution.
No build outputs are included. The registry source is unchanged.

The registry package declares MIT licensing but omits its license file.
`LICENSE.md` is the unchanged upstream file at the recorded commit:
<https://raw.githubusercontent.com/DioxusLabs/taffy/599b1b7d7410d97e13bafc4c01fbe6d7a9cf060a/LICENSE.md>.
Its 1,420 bytes have SHA-256
`a7083ef3dd41a65eeb2162c2c091b0082e2724fb4c6ed029b363c7840d244ecd`.
It preserves the original authors' MIT permissions and 2018 Visly copyright.

## Change

Only `src/compute/flexbox.rs` differs from the packaged runtime source.
`distribute_remaining_free_space` already subtracts the main-axis gaps before
assigning positive free space to auto margins. That branch omitted the
`offset_main` values which final placement uses to insert those gaps.
It now assigns one gap before every item except the first in visual order,
including row-reverse and column-reverse. Auto margins still consume all
remaining free space. Justification, zero/negative free space, wrapping,
cross-axis layout and the public API are unchanged.

Lumine depends directly on this directory with the original exact version pin,
disabled default features and existing feature list. A direct path works under
both the Ask workspace and Lumine's standalone workspace without mutating a
registry installation or depending on a consuming workspace's patch table.
Lumine excludes this vendor directory from workspace membership so workspace
builds do not enable Taffy's default features as an independent member.

`crates/lumine/tests/flex_auto_margin_gap.rs` checks row and column gaps with
one or two auto margins, both main-axis directions, and retained resizes through
positive, zero and negative free space. The parent performs execution and
lockfile updates.
