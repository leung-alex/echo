# Echo compatibility patch: Slint 1.17.1 accessibility

Origin: the pinned crates.io i-slint-backend-winit 1.17.1 package. Original copyright and license expressions are unchanged; upstream VCS metadata and LICENSES are retained.

`accesskit.rs` includes the small `echo_accessibility_value.rs` helper: choose string versus numeric accessibility value from the control role, not from parsing user text. Queries such as 0013 must preserve leading zeros and retain UIA ValuePattern. Actual sliders, spin buttons and progress indicators still expose numeric values. Two regression tests cover these cases.

This override is tracked via [patch.crates-io] in Echo, not in the global Cargo registry. Remove it only after a verified upstream upgrade passes these regressions and Echo's native input/edit tests. AboutSlint attribution remains available.
