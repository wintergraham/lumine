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

## Changes

`src/compute/flexbox.rs` and `src/compute/grid/track_sizing.rs` differ from
the packaged runtime source.

### Flex auto-margin gaps

`distribute_remaining_free_space` already subtracts the main-axis gaps before
assigning positive free space to auto margins. That branch omitted the
`offset_main` values which final placement uses to insert those gaps.
It now assigns one gap before every item except the first in visual order,
including row-reverse and column-reverse. Auto margins still consume all
remaining free space. Justification, zero/negative free space, wrapping,
cross-axis layout and the public API are unchanged.

### Cumulative grid track distribution

The upstream `distribute_space_up_to_limits` implementation at the same
recorded commit is preserved apart from its cumulative-growth calculations
and focused local tests:
<https://github.com/DioxusLabs/taffy/blob/599b1b7d7410d97e13bafc4c01fbe6d7a9cf060a/src/compute/grid/track_sizing.rs>.
Its active-track filter included `item_incurred_increase`, but the remaining
capacity and increment-acceptance checks omitted it. The application loop
also revisited tracks that had already reached their limits. Later rounds
could therefore exceed growth limits and spend more than the remaining
distribution budget.

For three tracks with bases 10, limits 11/13/16 and a budget of 10, following
the upstream loop gives increases 2/5/5 and a remainder of -2. The required
increases are 1/3/6 with no remainder. The patch subtracts accumulated growth
when computing remaining capacity, excludes frozen tracks when applying
increments, and checks each increment against the cumulative size. Existing
thresholds, weighted proportions and public APIs are unchanged.

This also explains the captured three-card failure exactly. Starting at 300
per row with limits 343.2/596/430 and 469.2 free pixels, the old rounds add
43.2, then 130, then 79.6. The second round exceeds the third row's remaining
86.8 capacity; the third round increments that frozen row again. The result
is 343.2/552.8/552.8, overspending by 79.6. Using remaining capacity instead
gives rounds of 43.2, 86.8 and 166, reaching the correct individual limits.

`distribution_tests` in that source file covers the cumulative/frozen case,
pre-existing increases, weighted and unaffected tracks, unused space after
all affected tracks freeze, and the distinct 343.2/596/430 card limits.
Lumine's existing
`auto_grid_rows_keep_individual_card_content_heights_at_the_natural_allocation`
Scene test exercises the measured-card callback and retained allocation path.
The production trace showed correct min/max-content contributions before
the finite-height distribution changed the individual row allocations.

### Dependency and validation

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
