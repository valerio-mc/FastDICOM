# FastDICOM guide

A macOS-first, general-purpose local DICOM viewer and native Rune framework playground.
Rust UI, no webview, no PACS networking, no cloud uploads.

**Experimental software, not validated for diagnostic interpretation.** This first
vertical slice is a 2D viewer, not a complete radiology workstation. MPR, 3D volume
rendering, measurements, linked viewports, and annotation persistence are
not implemented yet. They should be built on verified spatial geometry and an
evaluated GPU rendering path, not represented by decorative controls.

## Run

Rust 1.99.1 with the `aarch64-apple-darwin` target is required. The project selects
that target without changing your global Rust default. Rune is pinned to
`6354c31b0fa6c94a86e534cabdfed972228e6c35`.

```sh
cargo run --locked
# Or open a folder at startup:
cargo run --locked -- /path/to/study
```

Choose **Open folder** (⌘O) to index a local DICOM folder. For removable media with
a DICOMDIR, choose the containing folder: this build discovers files recursively,
rather than interpreting DICOMDIR references. Patient metadata stays in memory;
there is no study database or automatic file copying. Do not share screenshots
containing identifying metadata.

### Current format boundary

The initial decoder accepts Part 10 files (with a `DICM` marker), single-frame
MONOCHROME1/2, and aligned 8- or 16-bit integer samples. Native implicit/explicit
little-endian, explicit big-endian, common JPEG lossy/lossless, and bounded monochrome
RLE decoding are supported. Dataset/frame deflate and unreviewed transfer syntaxes
are explicitly rejected rather than decompressed without safe output bounds.
Color, enhanced/multiframe objects,
modality/VOI/presentation LUT sequences, and non-LINEAR VOI functions are not yet
supported. Slope/intercept rescale and MONOCHROME1 polarity are respected.

Files are capped at 256 MiB; images at 16 million pixels and 8192 pixels per side;
indexing at 100,000 directory entries. Symlinks are not traversed. Images are ordered
by patient-space position when all instances have valid consistent orientation,
otherwise by InstanceNumber/path. The viewer does not yet retain full spatial
geometry for MPR, orientation labels or calibrated
measurements. Do not infer physical dimensions or anatomical direction from the
display. Pixel spacing/aspect metadata is retained to preserve the displayed image's
physical proportions; this does not provide measurement calibration.

An immutable file snapshot is preflighted with lazy parser tokens before eager
metadata parsing. Element lengths, nesting, metadata allocation and compressed
output are bounded. These checks are not a security certification or a guarantee
against every dependency vulnerability.

The synthetic demo is a mathematically generated phantom, not a patient study or
an anatomically accurate teaching image. It is explicitly marked in the viewer.

## Viewing

- Scroll over the image to change slices; use arrow keys with the viewport focused.
- Select Window/level, Pan, or Zoom, then drag over the image.
- Pinch to zoom. Fit resets image geometry; Reset restores default window/level too.
- Invert and rotate operate on the current image presentation.
- Auto contrast runs by default when opening a series. The histogram icon (**A**) recalculates it for the active
  pane. CT series also expose a native preset menu: soft tissue **400/40**, lung
  **1500/−600**, and bone **2000/400** (width/level, modality-rescaled values).
- Select a series in the sidebar to load it.
- Toggle side-by-side panes using the split button in the bottom dock. Click a pane
  to activate it; dock controls and keyboard shortcuts affect that pane only.
  Drag a series preview into either pane to load it there, or activate a pane then
  click a series. Drag the central divider (or focus it and use arrow keys) to resize
  within 30–70%, retaining at least 300 points per pane. Turning split off
  keeps the active pane, including its presentation and pending request.
  Series dragging stays inside the viewer: release over a pane to load it, or press
  Escape/release outside the panes to cancel. A short click still selects a series.
  Image shortcuts follow the active pane after scrolling, pinching or dropping,
  even if keyboard focus is still elsewhere in the viewer.
- Each pane has independent slices, window/level, pan, zoom, inversion and rotation.
  Patient details and Privacy mode are shared. Loading a different patient clears
  the other pane. PatientID/issuer and available names must match; without IDs,
  comparison is limited to the same study and matching name. This is a conservative
  guard, not patient-identity reconciliation. Inconsistent patient metadata within
  one study/series UID is skipped on import.
- Series previews use the middle image of each series and load progressively in
  the background; image navigation does not wait for the preview batch.

The bottom-center icon dock provides Window/level, Pan, Zoom, Fit, Reset, Invert,
and clockwise rotation. Hover for action names and shortcuts. Glass surfaces share
Rune FX's screen-space lens material: backdrop refraction, frosted transmission,
rim lighting, and contact shadows; chromatic dispersion is disabled to avoid pink
fringes. All surfaces use the dock's green tint. A compact unified title strip combines native macOS window
buttons with the series toggle (⌘B), Open folder, app identity, and Privacy mode.
Series previews float without an enclosing sidebar panel. Individual
selected/hovered rows use the dock's in-app glass and a
subtle green tint. Patient details float at the top right without a resting panel;
hovering reveals the same material. The window and its
continuous workspace backing are fully opaque. Glass effects sample app content,
never the desktop; the image canvas remains fully opaque too.
Skipped-file counts are available in the Open folder tooltip, not a footer strip.

The **eye button** in the title strip (⇧⌘P) hides patient names, study/series descriptions,
other imported text, and detailed error messages from the interface and its
accessibility labels. An open eye means details are visible; a slashed eye means
they are hidden. Click again to restore the information. It is session-local
and stays set when opening another folder. It does not alter or anonymize DICOM
files, remove identifying text burned into image pixels, hide native file-picker
paths, or erase metadata from memory. Check image pixels before sharing a screen.

Indexing and image decoding run on Rune's bounded background worker pool.
Requests are latest-wins so obsolete results cannot replace a newer
slice. A 128 MiB decoded-frame cache avoids repeat decoding for recently visited
images, shared across both panes and keyed by file path. Each pane owns one resident
GPU texture; only a changed image is uploaded. A custom Rune compositor uploads raw grayscale samples and applies
window/level and inversion in a GPU shader; changing these controls does not
re-decode or re-upload the image. End-to-end latency, FPS, and display refresh
performance have not been benchmarked.

### Automatic contrast

Auto contrast is enabled by default for both MRI and CT, even when a DICOM window
is supplied. The first image gets an immediate robust intensity window; up to five
evenly spaced slices are then sampled sequentially in the background to establish
one stable series window. The Auto action explicitly recalculates this estimate
after manual adjustments or preset selection. Completed estimates are
reused within the currently opened folder. A new folder or demo clears them.

Each sampled frame contributes at most 8,192 evenly distributed **valid** samples
(at most 40,960 combined). The lowest and highest 1% are clipped when choosing the
window. Declared PixelPaddingValue/PixelPaddingRangeLimit is excluded in the signed
or unsigned stored-value domain before modality rescaling. Invalid padding metadata
is rejected. Undeclared zero-valued pixels are **not** removed. Neither padding nor
outliers are rewritten in the displayed pixel data. MRI is estimated from its own
intensity distribution, with no assumed Hounsfield-unit scale.

Scrolling preserves the chosen window, whether supplied, automatic, preset or
manually adjusted. Manual window changes, Reset and CT presets cancel pending
automatic application; late results cannot overwrite them or affect another pane.
Replacing a series cancels its request; an in-progress decode may finish, but the
sampler stops before the next slice and its stale result is ignored. A failed sample
leaves the current image/window available and reports that Auto is unavailable.

Percentile windows and named CT presets are convenience heuristics, not diagnostic
recommendations or guarantees of visible anatomy. Uniform/all-padding samples use
a finite width of at least one; an all-padding image contains no useful contrast.
Always verify the presentation for the clinical task and acquisition.

A thin indeterminate progress bar appears at the top edge of the image viewport
while indexing or loading a slice. It does not invent a percentage when the decoder
cannot report one. The prior slice stays visible while the next is prepared.
The animation uses Rune's renderer clock and respects reduced motion.

## Build the application

```sh
zsh scripts/bundle.sh
open dist/FastDICOM.app
```

The script builds an Apple Silicon release application with an app icon and local
ad-hoc signing. It is not notarized or prepared for redistribution. It does not
install into `/Applications` or replace any other app.

On this Desktop, File Provider reattaches Finder metadata after signing, so a later
strict `codesign --verify` check can fail despite successful local launch. Persistent
strict signature validation is not established; distribution signing/notarization
is outside this experimental build.

## Verification

Tests live in `tests/unit/` and remain private unit-test modules of the application;
they do not require exposing application internals as a library API.

```sh
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo run --locked -- --scene demo --snap /absolute/scratch/fastdicom.png
```

Native-window behavior, VoiceOver, color/display calibration and representative
real-world transfer syntaxes require testing on actual studies and hardware.
