# Kindle Paperwhite port

Status: feasibility note, deferred until after the current exams.

This records the investigation from 2026-07-29 into running Idiosepius
directly on an unsupported first- or second-generation Kindle Paperwhite. The
short conclusion is that this is realistic. The preferred route is a small,
original Rust backend for those two devices, not a dependency on FBInk and not
a general e-reader compatibility library.

No Kindle code has been written yet. The exact device generation, firmware and
userspace ABI are still unknown and must be identified before choosing a Rust
target or issuing display updates.

## Decision

Build a dedicated Kindle shell around the existing application:

```text
Kindle touch device
        |
        v
Linux evdev events
        |
        v
egui RawInput -> Idiosepius UI -> tessellated egui meshes
                                      |
                                      v
                              CPU software renderer
                                      |
                                      v
                           grayscale shadow framebuffer
                                      |
                         changed tiles / refresh rectangles
                                      |
                                      v
                    mmap(/dev/fb0) + Kindle MXCFB ioctl
                                      |
                                      v
                                  e-ink panel
```

The work is three separate problems:

1. Run egui without a desktop window or GPU.
2. Copy finished grayscale pixels into the Linux framebuffer and ask the
   Kindle display controller to refresh them.
3. Translate Linux touch events into egui pointer events.

Only the second part overlaps FBInk. Because Idiosepius already renders its own
fonts, images, maths, plots and rotated cards, it needs none of FBInk's text,
font, image-decoding, layout or multi-device functionality.

## Why not FBInk

[FBInk](https://github.com/NiLuJe/FBInk) is an impressive compatibility layer
for Kindles, Kobos and several other e-readers. It handles many framebuffer
formats, display-controller generations, rotations, waveform quirks, fonts,
image formats and dithering paths. That breadth is why it is useful, but almost
all of it is outside this port's scope.

FBInk is GPLv3+. Linking it into the MIT-licensed Idiosepius binary would make
distribution terms needlessly complicated. Its C API would also add a
cross-language build and FFI boundary at the lowest level of the application.
Calling the FBInk executable as a separate process might create a different
licensing situation, but would still be an awkward rendering protocol and is
not the proposed design.

The Rust implementation must be original code against the public userspace
ABI. Translating or mechanically porting FBInk does not remove its copyright
or GPL obligations. The relevant ABI should be established from the kernel
userspace headers corresponding to the device firmware and confirmed by
on-device probes. Existing projects are useful behavioural references, but
their implementation should not be copied. This is an engineering boundary,
not legal advice; the licensing of the exact old vendor headers should be
checked before publishing a permissively licensed crate.

## Scope of the Rust display backend

Do not attempt to create "FBInk in Rust". Start with only PW1 and PW2, an 8-bit
grayscale source image, portrait orientation, and rectangular updates.

The backend needs roughly this surface:

```rust
struct Framebuffer {
    // device file, mapping, geometry, stride, format, model and update marker
}

impl Framebuffer {
    fn open(path: &Path) -> Result<Self>;
    fn info(&self) -> FramebufferInfo;
    fn blit_gray8(&mut self, rect: Rect, pixels: &[u8], source_stride: usize)
        -> Result<()>;
    fn refresh(&mut self, rect: Rect, waveform: Waveform, mode: UpdateMode)
        -> Result<UpdateMarker>;
    fn wait(&self, marker: UpdateMarker) -> Result<()>;
}
```

Estimated size is 500–1,000 lines for a careful display implementation with
validation and tests, plus a few hundred for input and process lifecycle. The
uncertainty lies in hardware behaviour, not code volume.

### Standard Linux framebuffer operations

The ordinary framebuffer interface provides most of what is needed:

- open `/dev/fb0`;
- query fixed and variable screen information with
  `FBIOGET_FSCREENINFO` and `FBIOGET_VSCREENINFO`;
- validate resolution, virtual resolution, pixel depth and pixel format;
- map `smem_len` bytes;
- use the reported `line_length`, never `width * bytes_per_pixel`, as the row
  stride;
- account for `xoffset` and `yoffset`;
- copy only clipped, validated rows into the mapping.

The upstream [Linux framebuffer userspace
documentation](https://docs.kernel.org/fb/api.html) is the reference for this
part. The first probe must print every relevant field instead of assuming the
widely reported 758 × 1024, 8-bpp layout.

Keep the framebuffer mapping and Kindle refresh operations in one crate. No
other application module should know about file descriptors, `mmap` or
`ioctl`.

### Kindle refresh operations

Writing framebuffer memory does not by itself cause a useful e-ink update. The
Kindle's i.MX EPDC driver accepts an update request containing:

- a rectangle: top, left, width and height;
- a waveform mode;
- partial or full update mode;
- a non-zero update marker;
- temperature and flags;
- histogram waveform choices;
- an unused alternate-buffer description.

The exact C-compatible layout and ioctl encoding must come from the kernel
headers for the installed firmware. Preliminary research indicates that PW1
and PW2 share the same basic update request but differ in the completion wait:

- PW1/Pearl waits using a plain 32-bit update marker;
- PW2/Carta waits using a marker structure containing the marker and collision
  result.

Both variants use ioctl group `F`; preliminary operation numbers are `0x2e`
for submitting an update and `0x2f` for waiting. Treat those as findings to
verify, not magic numbers to paste into code. Generate ioctl request values
from the operation, direction and `#[repr(C)]` argument type so the encoded
size is correct on 32-bit ARM.

Required defensive rules:

- compile-time and test-time checks for every ABI type's size and alignment;
- only fixed-width integer fields in vendor structures;
- clamp every rectangle to the visible framebuffer;
- reject empty rectangles and rectangles with width or height at most one;
- use a monotonically increasing, non-zero update marker;
- preserve and report `errno`;
- put a timeout around completion waits at the runner level;
- begin with a full-screen, high-quality update;
- do not experiment with unverified update flags on the user's study database
  or normal Kindle UI.

The first implementation needs only these conceptual waveform choices:

- `GC16`: slow, good grayscale, suitable for settled screens and clearing
  ghosting;
- `DU`: fast black/white update for small interaction changes;
- possibly `A2`: very fast monochrome interaction after the basic route is
  proven;
- `AUTO`: not necessary for the first probe.

Likely kernel values observed in existing implementations are `DU = 1`,
`GC16 = 2`, `A2 = 4`, partial update `= 0`, full update `= 1`, and automatic
temperature `= 0x1001`. All must be checked against the device's own kernel
source/header before use.

The hardware model can probably be determined from `/proc/usid`, but the
prototype should also allow an explicit `--model pw1|pw2`. Silent guessing is
worse than refusing to refresh.

## egui without eframe's desktop runner

`eframe::run_native` supplies a window, winit event loop and GPU renderer. None
of those belongs on this Kindle. The Kindle binary should drive
`egui::Context` directly:

1. Gather touch/key events and timing into `egui::RawInput`.
2. Call `Context::run`.
3. Build the root `Ui` and call the same Idiosepius screen code used by eframe.
4. Tessellate the returned shapes into `ClippedPrimitive` meshes.
5. Apply texture deltas.
6. Rasterize into a memory buffer.
7. Convert to grayscale, find changed regions, blit and refresh.
8. Sleep until input or the next requested repaint deadline.

The application currently implements `eframe::App::ui` directly and imports
egui through `eframe::egui`. A port will probably first:

- add a direct `egui = 0.35` dependency;
- move the existing root UI body into an eframe-independent method;
- leave a small `eframe::App` adapter for desktop;
- add a separate Kindle runner which calls that method;
- keep the browser adapter unchanged.

This should be a structural extraction, not a fork of the screens. There must
still be one implementation of card layout, explanation rules, scheduler
interaction and database requests.

### Software rendering

[`egui_software_backend`](https://docs.rs/egui_software_backend/latest/egui_software_backend/)
is the strongest starting point found. It renders ordinary egui meshes on the
CPU, supports texture deltas, caches rasterized primitives, and maintains
64-pixel dirty tiles internally. It is MIT/Apache-2.0 licensed.

The published 0.0.3 crate currently depends on egui 0.34 while Idiosepius uses
0.35, so expect either a small upstream update or a maintained fork. Its one
important omission is paint callbacks. Idiosepius currently uses normal egui
painting rather than `PaintCallback`, so maths, plots and manually rotated
cards should remain meshes and work.

The dirty tile mask is not currently exposed as a public result. Options are:

- expose the renderer's dirty tiles in the small version-update fork; or
- retain the previous Gray8 frame and compare output in 32- or 64-pixel tiles.

The second option is simple and renderer-independent. A full comparison is
less than one megabyte per rendered frame at PW1/PW2 resolution, and settled
screens should render no frames at all.

Render initially to RGBA even though the display is grayscale. That keeps egui
texture blending straightforward. Convert each output pixel to linear or
perceptual luminance, quantize deliberately, then compare against the Gray8
shadow buffer. Direct Gray8 rendering is an optimization for later.

The proof in
[`Quill-OS/egui-fbink`](https://github.com/Quill-OS/egui-fbink) establishes
that egui can be driven on this class of device, but it is not an application
backend to adopt. It targets egui 0.27, handles selected high-level shapes
itself, delegates text to FBInk, ignores several mesh/curve/callback forms and
does not implement real input. Idiosepius needs the final tessellated meshes
rendered faithfully instead.

### Partial redraw

egui does not promise a framebuffer damage region. Discussion
[#913](https://github.com/emilk/egui/discussions/913) recommends solving this
in the backend by comparing or caching clipped paint output. That is exactly
where the Kindle runner should do it.

Start with one bounding rectangle around all changed tiles. Later, coalesce
neighbouring tiles into a small number of rectangles if reducing refreshed
area matters more than issuing extra ioctls. Force a full-screen refresh when:

- the application starts or regains the screen;
- navigation replaces most of the page;
- the accumulated partial-update count reaches a tuned threshold;
- the changed-area ratio is high;
- visible ghosting demands it;
- the application exits back to the Kindle UI.

The renderer's damage calculation and the panel waveform policy are separate:
the former says what changed; the latter says how faithfully and quickly to
refresh it.

## Touch input

The touchscreen is exposed through Linux evdev, commonly as a `cyttsp` device
on this generation. Do not hardcode `/dev/input/event0`. Enumerate event
devices, print their names and capabilities, and choose the device which
reports absolute touch coordinates.

Translate a contact into egui events:

- absolute X/Y to `PointerMoved`;
- contact begin/end to primary `PointerButton`;
- optional multitouch slot/tracking events reduced to the first active contact;
- hardware keys, if useful, to egui key events;
- `SYN_REPORT` as the boundary at which one coherent input sample is emitted.

The probe must record raw min/max axes and discover whether portrait
coordinates need swapping or inversion. Map through the advertised axis range,
not a presumed screen resolution. Input rotation belongs beside framebuffer
rotation in the device backend.

The complete first version only needs one finger. Pinch zoom, gestures from the
Kindle framework and pressure are out of scope.

## An e-ink-specific UI mode

The existing deep-water palette is intentionally dark and animated. Rendering
most of the screen dark on e-ink would be slow, ghost-prone and visually less
pleasant. A Kindle mode therefore needs a high-key palette while preserving
the rules in `DESIGN.md`:

- white or near-white page;
- a few exact grayscale surface levels;
- black or near-black prose;
- crisp rectangular rules;
- no shadows and no rounded corners;
- tracked monospace chrome;
- semantic differences backed by labels, glyphs or stroke patterns, never
  grayscale alone.

Cyan/violet swipe direction and green/magenta verdicts cannot survive a
grayscale panel literally. Their meanings can:

- `TRUE` and `FALSE` remain printed on the gesture;
- opposing outline weights or hatch directions distinguish swipe sides;
- correct/wrong feedback includes an unambiguous word or symbol;
- the grayscale levels are chosen for contrast, not as pretend colours.

This should be an explicit e-ink theme selected by the Kindle shell, not a
change to the desktop and web design. Issue
[#4663](https://github.com/emilk/egui/issues/4663) contains relevant egui
discussion and examples of deliberately constrained e-ink palettes.

Animation also needs an explicit policy:

- hide the ocean background;
- disable the coin spin and entry/hover transitions;
- prefer drawing once on touch release over repainting every drag sample;
- if live swipe feedback is kept, rate-limit it and use `DU` or `A2`;
- use `GC16` for the stable question and feedback state;
- never run a desktop-style continuous frame loop.

This is similar to screenshot mode's requirement that all animation be
controllable, but for latency, ghosting and power rather than reproducibility.

## Application-shell responsibilities

The Kindle runner replaces parts of eframe, not only its renderer. It must
handle:

- viewport size and pixels-per-point;
- clock and repaint deadlines;
- touch and key input;
- clipboard requests, or explicitly report them unsupported;
- database and settings paths;
- app import/export requests;
- clean suspension, wake and exit;
- ownership of the screen relative to Amazon's framework;
- error reporting when no terminal is visible.

For the first vertical slice, use a pre-populated database at a fixed
application-owned path. Native `rfd` dialogs will not exist on the Kindle.
Import/export can initially be command-line or a documented USB directory.
Later, the Kindle shell can implement the application's existing request model
without adding file handling to the deck screen.

Running alongside Amazon's graphical framework risks both processes painting
the framebuffer. The launcher will probably need to suspend or stop the
Lab126 UI while Idiosepius owns the panel, then restore it reliably on normal
exit, error and signal. Exact commands are firmware-specific and should not be
written until the device is identified. Package the result as a KUAL extension
only after the manual SSH-launched binary is reliable.

## 32-bit ARM and database concerns

The application's successful `wasm32` build is strong evidence that its own
data structures and UI logic do not inherently require 64-bit pointers.
However, WebAssembly and native Linux select different dependency trees and
system ABIs.

A research build of the current workspace for
`armv7-unknown-linux-gnueabihf` found two native dependency problems:

1. `io-uring 0.7.13` rejects the architecture in its generated bindings unless
   its architecture check is bypassed.
2. After bypassing that check, `turso_sync_engine` reaches calls where 64-bit
   SQLite offsets do not match the target's 32-bit `off_t` declarations.

This does not prove that the Kindle port needs a database-driver change. The
likely deployment target is a statically linked ARMv7 musl target, whose ABI
may differ, and unused Turso features may be removable. It does mean that
"Wasm is 32-bit" is not sufficient validation for the native dependency path.

When the device is available, collect:

```sh
uname -a
cat /proc/cpuinfo
cat /proc/usid
file /bin/busybox
readelf -A /bin/busybox
readelf -l /bin/busybox
```

Then try a statically linked hello-world program before compiling Idiosepius.
`armv7-unknown-linux-musleabihf` is a plausible first target, not yet a
decision.

If Turso still blocks the real target, investigate in this order:

1. remove native features and dependencies which the local synchronous store
   does not use;
2. use `turso_core` directly for the Kindle target, as the Wasm build already
   does;
3. patch the pure-Rust Turso offset handling upstream or locally;
4. only then consider a Kindle-specific `rusqlite` implementation behind
   `sql.rs`.

The database façade deliberately confines such a change. Do not spread a
Kindle database workaround into the scheduler or application.

## Proposed repository shape

One reasonable layout, to be adjusted after the probes:

```text
crates/
    app/                    existing shared UI
    core/                   existing database and study logic
    kindle-device/          fbdev, MXCFB, model detection and evdev
    kindle-runner/          egui loop, software rasterizer and app shell
tools/
    kindle-probe/           or a small binary in kindle-device
kindle/
    KUAL launcher/package files, added only after SSH testing
```

It may be simpler to begin with `kindle-device` and one probe binary, and add
the runner crate only after the panel and input APIs are known.

## Implementation sequence

### Phase 0: identify and preserve the device

- Determine whether it is PW1 or PW2 from model/serial and firmware.
- Confirm a supported jailbreak and SSH route.
- Back up anything worth keeping.
- Record kernel, CPU, framebuffer, input and dynamic-loader information.
- Confirm how to return to the stock UI after a failed foreground program.

Do not begin by stopping services or issuing undocumented display ioctls.

### Phase 1: Rust hardware probe

Build a standalone binary with no egui or database dependencies. It should:

1. identify the model;
2. print framebuffer fixed/variable information;
3. list evdev devices and absolute-axis ranges;
4. map `/dev/fb0`;
5. save the original visible framebuffer in memory;
6. draw a grayscale bars/checkerboard test;
7. perform one full-screen `GC16` refresh;
8. perform one small `DU` refresh;
9. wait for both updates with a timeout;
10. restore the saved pixels and perform a final full refresh;
11. print touch contacts and mapped screen coordinates.

The probe should default to inspection only and require an explicit flag before
writing or refreshing.

### Phase 2: egui renderer spike on the development machine

- Update/fork the software backend to egui 0.35.
- Feed it Idiosepius's deterministic screenshot screens.
- Compare its output to the existing screenshot route.
- Verify text, imported fonts, maths, plots, images and rotated cards.
- Expose or independently calculate changed tiles.
- Add a grayscale conversion and e-ink palette preview.

This phase needs no Kindle and can catch most rendering incompatibilities.

### Phase 3: static egui screen on Kindle

- Cross-compile the runner without the database.
- Render one representative Idiosepius screenshot.
- Copy its pixels and refresh with `GC16`.
- Measure render time, memory usage and refresh time.
- Validate orientation, grayscale order and clipping.

### Phase 4: interactive shell

- Feed touch into egui.
- Implement repaint scheduling.
- Exercise navigation and scrolling with dummy state.
- Tune partial-update rectangles and waveform selection.
- Verify that a crash or signal restores a readable stock screen.

### Phase 5: complete application

- Resolve the native Turso target.
- Open a copied test database.
- Extract the common UI entry point from the eframe adapter.
- Handle settings and app requests.
- Test a complete study session, feedback, undo, lessons and restart.
- Confirm that copying the database off the device preserves all history.

### Phase 6: packaging

- Add a KUAL launcher.
- Make screen ownership and restoration idempotent.
- Store data outside the application bundle.
- Document installation, update, database copy and recovery.
- Keep the desktop, web and Kindle entry points independently buildable.

## Acceptance criteria

The first useful port is complete when:

- it launches repeatedly from the device without manual recovery;
- all representative UI screens render without missing primitives;
- touch targets and scrolling are accurate across the whole panel;
- idle screens consume no continuous CPU;
- common interactions do not require full-screen flashes;
- periodic full refreshes prevent cumulative ghosting;
- killing the process returns a readable stock Kindle UI;
- a real copied database survives study, undo, exit and restart;
- the same database remains readable by desktop Idiosepius and `sqlite3`;
- no GPL library is linked or incorporated into the shipped binary;
- desktop and browser behaviour remains unchanged.

## Main risks and open questions

- **Exact hardware:** PW1 and PW2 use related but not identical EPDC completion
  ABIs. First identify the unit.
- **Jailbreak and launcher:** firmware version determines the safe route.
- **Vendor ABI provenance:** locate the matching kernel release/header and
  check its licensing before publishing definitions.
- **Framebuffer format:** verify 8-bpp grayscale, stride, offsets and polarity.
- **Rotation:** determine whether the framebuffer or touch device reports
  native or logical portrait coordinates.
- **Refresh safety:** confirm struct layout and ioctl sizes on 32-bit ARM
  before submitting an update.
- **Ghosting policy:** tune waveform and full-refresh cadence on the actual
  panel, not from screenshots or another device.
- **Software-renderer maintenance:** egui 0.35 support and dirty-tile exposure
  may require a small fork.
- **Performance:** measure CPU rasterization and RGBA-to-gray conversion on the
  device; do not optimize speculatively.
- **Memory:** the framebuffer, RGBA render target, Gray8 shadow and renderer
  caches should fit comfortably in principle, but measure peak RSS.
- **Native Turso:** the current GNU ARM cross-check fails in dependencies.
- **Stock UI ownership:** Amazon's framework must not repaint over the app, and
  must always be restored.
- **File operations:** there is no native file picker; define a Kindle shell
  behaviour for import/export.
- **Network imports:** old TLS roots and firmware networking may make GitHub
  imports unreliable. They are not required for the first version.
- **Semantic colour:** direction and verdict meanings need redundant
  monochrome cues.

## References

- [egui e-ink theming issue #4663](https://github.com/emilk/egui/issues/4663)
- [egui Kindle/FBInk issue #4456](https://github.com/emilk/egui/issues/4456)
- [egui partial redraw discussion #913](https://github.com/emilk/egui/discussions/913)
- [Quill OS egui-fbink proof](https://github.com/Quill-OS/egui-fbink)
- [egui software backend documentation](https://docs.rs/egui_software_backend/latest/egui_software_backend/)
- [FBInk](https://github.com/NiLuJe/FBInk)
- [KOReader porting notes](https://koreader.rocks/doc/topics/Porting.md.html)
- [Linux framebuffer userspace API](https://docs.kernel.org/fb/api.html)

These references establish feasibility and identify the relevant interfaces.
They are not substitutes for the kernel headers and observed behaviour of the
actual Kindle.
