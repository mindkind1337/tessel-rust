# Tessel patch to vt100 0.15.2

Source: crates.io vt100 0.15.2, MIT, Jesse Luehrs. The upstream package sources,
manifest and license are retained here; Cargo patches the pinned dependency to
this directory. No global Cargo registry files are modified.

`Grid::visible_rows` previously subtracted the scrollback offset from the screen
height without saturation. Scrolling more than one screen back panicked in debug
builds; release builds returned more rows than the screen could display.

The patch caps archived rows to the screen height and saturates the count of
remaining live rows. Other parsing, input, sizing and formatting are unchanged.
The deprecated `u8::max_value()` spelling is also replaced by the equivalent
`u8::MAX` constant to keep current Rust compiler diagnostics clean.

`Parser::use_primary_screen_for_archive` exposes the existing internal grid
switch for an old parser that is about to be discarded. Tessel copies alternate
cells first, then reads the primary screen/history directly. This avoids parsing
a synthetic escape while remote output may have left an unfinished OSC/DCS.

Regression coverage lives in `tests/host_terminal_viewport.rs`, including a
second screen replacement while scrolled into archived history. Keep explicit
deep-scrollback row-count and content checks when upgrading this dependency.
