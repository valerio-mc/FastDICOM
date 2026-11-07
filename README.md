# FastDICOM

**Open a folder. Explore a study. Close it.**

A small, native DICOM viewer for macOS. No library to manage, no database to
maintain, no cloud uploads—just your images and the tools to explore them.

Built in Rust with [Rune](https://github.com/anomalyco/rune), without a webview.
An experiment in making medical-image viewing feel simple, local, and immediate.

**Experimental software. Not validated for diagnostic interpretation.**

## Try it

On Apple Silicon macOS, with the pinned Rust toolchain:

```sh
cargo run --locked -- /path/to/dicom-folder
```

Or run without a path and choose **Open folder** or **Synthetic demo**.

See the [guide](docs/guide.md) for controls, supported formats, privacy limits,
local app bundling, and verification commands.
