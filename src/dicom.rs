//! Blocking, local-only DICOM loading. Call scan/decode on a worker, not the UI thread.
//! A DICOMDIR selection scans its containing directory recursively; directory-record
//! links are not interpreted. Symlinks are not followed. Sorting uses patient-space
//! slice position only when every instance has valid, consistent orientation;
//! otherwise the entire series falls back to InstanceNumber, then pathname.

use dicom_object::{DefaultDicomObject, OpenFileOptions, Tag};
use dicom_parser::dataset::{LazyDataToken, lazy_read::LazyDataSetReader};
use dicom_parser::stateful::decode::StatefulDecode;
use dicom_pixeldata::PixelDecoder;
use dicom_transfer_syntax_registry::{TransferSyntax, TransferSyntaxIndex, TransferSyntaxRegistry};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PIXELS: usize = 16 * 1024 * 1024;
const MAX_SIDE: u32 = 8192;
const MAX_FILES: usize = 100_000;
const MAX_ELEMENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOKENS: usize = 100_000;
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone)]
pub struct Series {
    pub id: String,
    pub description: String,
    pub modality: String,
    pub study_description: String,
    pub patient_name: String,
    pub patient_id: String,
    pub patient_issuer: String,
    pub files: Vec<PathBuf>,
}

impl Series {
    pub fn same_patient(&self, other: &Self) -> bool {
        if !self.patient_id.is_empty() && !other.patient_id.is_empty() {
            self.patient_id == other.patient_id
                && self.patient_issuer == other.patient_issuer
                && (self.patient_name.is_empty()
                    || other.patient_name.is_empty()
                    || self.patient_name == other.patient_name)
        } else {
            self.id.split('/').next() == other.id.split('/').next()
                && self.patient_name == other.patient_name
        }
    }
}

#[derive(Debug)]
pub struct ScanResult {
    pub series: Vec<Series>,
    /// Files/symlinks rejected as non-images, unreadable, oversized, or invalid.
    pub skipped: usize,
}

#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Physical row spacing / column spacing (height per pixel / width).
    pub pixel_aspect: f32,
    /// Modality-rescaled values, before VOI windowing or photometric inversion.
    pub values: Vec<f32>,
    pub window: f32,
    pub level: f32,
    /// Whether window/level came from supplied DICOM VOI metadata rather than robust fallback.
    pub supplied_window: bool,
    /// At most 8192 evenly distributed valid samples, excluding declared stored-domain padding.
    pub contrast_samples: Vec<f32>,
    /// True for MONOCHROME1. `window_rgba` applies this automatically.
    pub inverted: bool,
}

#[derive(Debug)]
struct Instance {
    path: PathBuf,
    number: Option<i32>,
    position: Option<[f64; 3]>,
    orientation: Option<[f64; 6]>,
}

/// Recursively index a folder (or the parent of a selected DICOMDIR).
/// Folder traversal failures abort rather than presenting an incomplete index.
/// Non-image/invalid files are counted in `skipped`, not silently accepted.
pub fn scan_folder(path: &Path) -> Result<ScanResult, String> {
    let root = if path.is_file()
        && path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("DICOMDIR"))
    {
        path.parent().ok_or("DICOMDIR has no parent folder")?
    } else {
        path
    };
    if !root.is_dir() {
        return Err(format!("Not a folder or DICOMDIR: {}", path.display()));
    }
    let mut pending = vec![root.to_path_buf()];
    let mut groups: BTreeMap<(String, String), (Series, Vec<Instance>)> = BTreeMap::new();
    let mut skipped = 0;
    let mut visited = 0;
    while let Some(folder) = pending.pop() {
        let entries =
            fs::read_dir(&folder).map_err(|e| format!("Cannot read {}: {e}", folder.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("Cannot read directory entry: {e}"))?;
            let kind = entry
                .file_type()
                .map_err(|e| format!("Cannot inspect {}: {e}", entry.path().display()))?;
            visited += 1;
            if visited > MAX_FILES {
                return Err(format!(
                    "Folder exceeds the {MAX_FILES}-entry indexing limit"
                ));
            }
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !kind.is_file() {
                skipped += 1;
                continue;
            }
            let file_path = entry.path();
            let obj = match read_object(&file_path, true) {
                Ok(obj) => obj,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };
            let study = text(&obj, "StudyInstanceUID");
            let uid = text(&obj, "SeriesInstanceUID");
            if study.is_empty() || uid.is_empty() || dimensions(&obj).is_err() {
                skipped += 1;
                continue;
            }
            let instance = Instance {
                path: file_path,
                number: obj
                    .element_by_name("InstanceNumber")
                    .ok()
                    .and_then(|e| e.to_int().ok()),
                position: array(&obj, "ImagePositionPatient"),
                orientation: array(&obj, "ImageOrientationPatient"),
            };
            let (series, instances) =
                groups
                    .entry((study.clone(), uid.clone()))
                    .or_insert_with(|| {
                        (
                            Series {
                                id: format!("{study}/{uid}"),
                                description: text(&obj, "SeriesDescription"),
                                modality: text(&obj, "Modality"),
                                study_description: text(&obj, "StudyDescription"),
                                patient_name: text(&obj, "PatientName"),
                                patient_id: text(&obj, "PatientID"),
                                patient_issuer: text(&obj, "IssuerOfPatientID"),
                                files: Vec::new(),
                            },
                            Vec::new(),
                        )
                    });
            if series.patient_id != text(&obj, "PatientID")
                || series.patient_issuer != text(&obj, "IssuerOfPatientID")
                || series.patient_name != text(&obj, "PatientName")
            {
                skipped += 1;
                continue;
            }
            instances.push(instance);
        }
    }
    let series = groups
        .into_values()
        .map(|(mut series, mut instances)| {
            sort_instances(&mut instances);
            series.files = instances.into_iter().map(|i| i.path).collect();
            series
        })
        .collect();
    Ok(ScanResult { series, skipped })
}

fn read_object(path: &Path, header_only: bool) -> Result<DefaultDicomObject, String> {
    let file = File::open(path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size > MAX_FILE_BYTES {
        return Err(format!("{} exceeds the 256 MiB file limit", path.display()));
    }
    // A bounded immutable snapshot makes the lazy preflight and eager parser see
    // exactly the same bytes. Read::take alone does NOT bound parser allocations:
    // an OB/UN header can declare gigabytes before any value bytes are read.
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("File exceeds the 256 MiB limit".into());
    }
    preflight_file(&bytes, header_only)?;
    let options = OpenFileOptions::new();
    let options = if header_only {
        options.read_until(Tag(0x7fe0, 0x0010))
    } else {
        options
    };
    options
        .from_reader(Cursor::new(&bytes))
        .map_err(|e| format!("Cannot parse {}: {e}", path.display()))
}

fn supported_syntax(uid: &str) -> bool {
    matches!(
        uid,
        "1.2.840.10008.1.2"
            | "1.2.840.10008.1.2.1"
            | "1.2.840.10008.1.2.2"
            | "1.2.840.10008.1.2.4.50"
            | "1.2.840.10008.1.2.4.51"
            | "1.2.840.10008.1.2.4.57"
            | "1.2.840.10008.1.2.4.70"
            | "1.2.840.10008.1.2.5"
    )
}

fn preflight_file(bytes: &[u8], header_only: bool) -> Result<(), String> {
    // Part 10 has one fixed-layout group-length element before its variable meta
    // elements. Validate that small envelope, then delegate all element/sequence
    // parsing to dicom-parser's allocation-free lazy reader (including file meta).
    let magic = if bytes.get(128..132) == Some(b"DICM") {
        128
    } else if bytes.get(..4) == Some(b"DICM") {
        0
    } else {
        return Err("A DICOM Part 10 file with a DICM marker is required".into());
    };
    let start = magic + 4;
    if bytes.get(start..start + 8) != Some(&[2, 0, 0, 0, b'U', b'L', 4, 0]) {
        return Err("Invalid File Meta Information Group Length header".into());
    }
    let length = bytes
        .get(start + 8..start + 12)
        .ok_or("Truncated file meta length")?;
    let meta_len = u32::from_le_bytes(length.try_into().unwrap()) as usize;
    let end = start + 12 + meta_len;
    if meta_len > MAX_METADATA_BYTES || end > bytes.len() {
        return Err("File meta length exceeds available bytes or metadata limit".into());
    }
    let explicit = TransferSyntaxRegistry.get("1.2.840.10008.1.2.1").unwrap();
    preflight_dataset(&bytes[start..end], explicit, false)?;
    let meta = dicom_object::FileMetaTable::from_reader(Cursor::new(&bytes[magic..end]))
        .map_err(|e| format!("Invalid file meta: {e}"))?;
    let uid = meta.transfer_syntax();
    // In particular, reject dataset deflate (.1.99) BEFORE a decompression reader
    // is installed, and encapsulated deflated frames (.8.1) BEFORE io::copy.
    // Other codecs must receive a deliberate pre-allocation review before use.
    if !supported_syntax(uid) {
        return Err(format!(
            "Transfer syntax {uid} is unsupported by the bounded loader (deflate and unreviewed codecs are disabled)"
        ));
    }
    let syntax = TransferSyntaxRegistry
        .get(uid)
        .ok_or("Unknown transfer syntax")?;
    preflight_dataset(&bytes[end..], syntax, header_only)
}

fn preflight_dataset(
    bytes: &[u8],
    syntax: &TransferSyntax,
    header_only: bool,
) -> Result<(), String> {
    let mut reader =
        LazyDataSetReader::new_with_ts(Cursor::new(bytes), syntax).map_err(|e| e.to_string())?;
    let mut depth = 0usize;
    let mut tokens = 0usize;
    let mut metadata = 0usize;
    while let Some(token) = reader.advance() {
        let token = token.map_err(|e| format!("Invalid DICOM structure: {e}"))?;
        tokens += 1;
        if tokens > MAX_TOKENS {
            return Err("DICOM token count exceeds safety limit".into());
        }
        let check_length = |len: u32, position: u64| -> Result<(), String> {
            if len as usize > MAX_ELEMENT_BYTES
                || position.saturating_add(u64::from(len)) > bytes.len() as u64
            {
                return Err(format!(
                    "Declared DICOM element length {len} exceeds available bytes or allocation limit"
                ));
            }
            Ok(())
        };
        match &token {
            LazyDataToken::ElementHeader(h) => {
                if header_only && depth == 0 && h.tag == Tag(0x7fe0, 0x0010) {
                    return Ok(());
                }
                let len = h.len.get().ok_or("Undefined primitive element length")?;
                if len as usize > MAX_ELEMENT_BYTES {
                    return Err("Declared DICOM element length exceeds allocation limit".into());
                }
            }
            LazyDataToken::LazyValue { header, decoder } => {
                check_length(header.len.0, decoder.position())?;
                if header.tag != Tag(0x7fe0, 0x0010) {
                    metadata = metadata
                        .checked_add(header.len.0 as usize)
                        .ok_or("Metadata length overflow")?;
                    if metadata > MAX_METADATA_BYTES {
                        return Err("DICOM metadata exceeds safety limit".into());
                    }
                }
            }
            LazyDataToken::LazyItemValue { len, decoder } => {
                check_length(*len, decoder.position())?
            }
            LazyDataToken::SequenceStart { len, .. } | LazyDataToken::ItemStart { len } => {
                if len.get().is_some_and(|len| len as usize > bytes.len()) {
                    return Err("Sequence/item length exceeds available bytes".into());
                }
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err("DICOM nesting exceeds safety limit".into());
                }
            }
            LazyDataToken::PixelSequenceStart => {
                if header_only && depth == 0 {
                    return Ok(());
                }
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err("DICOM nesting exceeds safety limit".into());
                }
            }
            LazyDataToken::SequenceEnd | LazyDataToken::ItemEnd => {
                depth = depth.checked_sub(1).ok_or("Unbalanced DICOM delimiters")?;
            }
            _ => return Err("Unsupported lazy DICOM token".into()),
        }
        token.skip().map_err(|e| e.to_string())?;
    }
    if depth != 0 {
        return Err("Truncated DICOM sequence/item".into());
    }
    Ok(())
}

fn text(obj: &DefaultDicomObject, name: &str) -> String {
    obj.element_by_name(name)
        .ok()
        .and_then(|e| e.to_str().ok())
        .map(|s| s.trim_matches([' ', '\0']).to_owned())
        .unwrap_or_default()
}

fn numbers(obj: &DefaultDicomObject, name: &str) -> Result<Option<Vec<f64>>, String> {
    let Some(element) = obj.element_by_name_opt(name).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let values = element
        .to_multi_float64()
        .map_err(|e| format!("Invalid {name}: {e}"))?;
    if values.is_empty() || values.iter().any(|v| !v.is_finite()) {
        return Err(format!("{name} must contain finite numbers"));
    }
    Ok(Some(values))
}

fn array<const N: usize>(obj: &DefaultDicomObject, name: &str) -> Option<[f64; N]> {
    numbers(obj, name).ok().flatten()?.try_into().ok()
}

fn integer(obj: &DefaultDicomObject, name: &str) -> Result<u32, String> {
    obj.element_by_name(name)
        .map_err(|e| format!("Missing {name}: {e}"))?
        .to_int::<u32>()
        .map_err(|e| format!("Invalid {name}: {e}"))
}

fn dimensions(obj: &DefaultDicomObject) -> Result<(u32, u32, usize), String> {
    let width = integer(obj, "Columns")?;
    let height = integer(obj, "Rows")?;
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(format!(
            "Image dimensions exceed the {MAX_SIDE}-pixel GPU texture side limit"
        ));
    }
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or("Pixel count overflow")?;
    if count == 0 || count > MAX_PIXELS {
        return Err(format!(
            "Image must have 1..={MAX_PIXELS} pixels (got {width}×{height})"
        ));
    }
    Ok((width, height, count))
}

fn pixel_aspect(obj: &DefaultDicomObject) -> Result<f32, String> {
    for name in [
        "PixelSpacing",
        "ImagerPixelSpacing",
        "NominalScannedPixelSpacing",
        "PixelAspectRatio",
    ] {
        if let Some(values) = numbers(obj, name)? {
            if values.len() != 2 || values.iter().any(|v| *v <= 0.0) {
                return Err(format!(
                    "{name} must contain exactly two finite positive spacings"
                ));
            }
            let aspect = (values[0] / values[1]) as f32;
            if !aspect.is_finite() || aspect <= 0.0 {
                return Err(format!(
                    "{name} ratio exceeds the positive finite f32 range"
                ));
            }
            return Ok(aspect);
        }
    }
    Ok(1.0)
}

fn normal(orientation: [f64; 6]) -> Option<[f64; 3]> {
    let [a, b, c, d, e, f] = orientation;
    if orientation.iter().any(|v| !v.is_finite())
        || (a * a + b * b + c * c - 1.0).abs() > 0.001
        || (d * d + e * e + f * f - 1.0).abs() > 0.001
        || (a * d + b * e + c * f).abs() > 0.001
    {
        return None;
    }
    Some([b * f - c * e, c * d - a * f, a * e - b * d])
}

fn sort_instances(instances: &mut [Instance]) {
    let geometry = instances
        .first()
        .and_then(|i| i.orientation)
        .and_then(|orientation| {
            let normal = normal(orientation)?;
            instances
                .iter()
                .all(|i| {
                    i.position.is_some_and(|p| p.iter().all(|v| v.is_finite()))
                        && i.orientation
                            .is_some_and(|o| normal_valid_and_close(o, orientation))
                })
                .then_some(normal)
        });
    instances.sort_by(|a, b| {
        let spatial = geometry
            .map(|n| {
                let distance = |i: &Instance| {
                    // Positions were validated for every instance above.
                    let p = i.position.unwrap();
                    p[0] * n[0] + p[1] * n[1] + p[2] * n[2]
                };
                distance(a).total_cmp(&distance(b))
            })
            .unwrap_or(std::cmp::Ordering::Equal);
        spatial
            .then_with(|| {
                a.number
                    .unwrap_or(i32::MAX)
                    .cmp(&b.number.unwrap_or(i32::MAX))
            })
            .then_with(|| a.path.cmp(&b.path))
    });
}

fn normal_valid_and_close(a: [f64; 6], b: [f64; 6]) -> bool {
    normal(a).is_some() && a.iter().zip(b).all(|(a, b)| (a - b).abs() <= 0.0001)
}

fn bounded_compressed_pixels(
    obj: &DefaultDicomObject,
    width: u32,
    height: u32,
    allocated: u32,
) -> Result<Option<Vec<u8>>, String> {
    let uid = obj.meta().transfer_syntax();
    if !uid.starts_with("1.2.840.10008.1.2.4.") && uid != "1.2.840.10008.1.2.5" {
        return Ok(None);
    }
    let element = obj
        .element_by_name("PixelData")
        .map_err(|e| e.to_string())?;
    let fragments = element
        .value()
        .fragments()
        .ok_or("Compressed transfer syntax requires encapsulated pixels")?;
    let size: usize = fragments.iter().map(|f| f.len()).sum();
    if size > MAX_ELEMENT_BYTES {
        return Err("Compressed pixel stream exceeds 64 MiB limit".into());
    }
    if uid == "1.2.840.10008.1.2.5" {
        if fragments.len() != 1 {
            return Err("Single-frame RLE requires exactly one fragment".into());
        }
        return decode_rle(
            &fragments[0],
            (width as usize) * (height as usize),
            allocated,
        )
        .map(Some);
    }
    // JPEG's own SOF dimensions, not DICOM Rows/Columns, govern its allocations.
    // Inspect the embedded header with the SAME codec/version as dicom-rs, before
    // allowing dicom-rs to decode. No JPEG image buffers are decoded by read_info.
    let stream: Vec<u8> = fragments.iter().flatten().copied().collect();
    preflight_jpeg(&stream, width, height, allocated)?;
    Ok(None)
}

fn preflight_jpeg(stream: &[u8], width: u32, height: u32, allocated: u32) -> Result<(), String> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(&stream));
    decoder.set_max_decoding_buffer_size(MAX_PIXELS * 2);
    decoder
        .read_info()
        .map_err(|e| format!("Invalid JPEG header: {e}"))?;
    let info = decoder.info().ok_or("Missing JPEG frame header")?;
    if u32::from(info.width) != width
        || u32::from(info.height) != height
        || !matches!(
            info.pixel_format,
            jpeg_decoder::PixelFormat::L8 | jpeg_decoder::PixelFormat::L16
        )
        || info.pixel_format.pixel_bytes() != allocated as usize / 8
    {
        return Err(
            "Embedded JPEG dimensions/sample layout disagree with bounded DICOM metadata".into(),
        );
    }
    Ok(())
}

/// Small bounded monochrome PackBits path. dicom-rs 0.9.1's RLE adapter first
/// expands each entire segment to an unbounded Vec, before its later `take`.
/// Here each packet is checked BEFORE resize/copy and output is pre-sized once.
fn decode_rle(fragment: &[u8], count: usize, allocated: u32) -> Result<Vec<u8>, String> {
    if fragment.len() < 64 {
        return Err("Truncated RLE header".into());
    }
    let word = |offset: usize| {
        u32::from_le_bytes(fragment[offset..offset + 4].try_into().unwrap()) as usize
    };
    let segments = word(0);
    let bytes_per_sample = allocated as usize / 8;
    if segments != bytes_per_sample {
        return Err("RLE segment count does not match monochrome sample layout".into());
    }
    let mut out = vec![0; count * bytes_per_sample];
    for segment in 0..segments {
        let start = word(4 + segment * 4);
        let end = if segment + 1 == segments {
            fragment.len()
        } else {
            word(8 + segment * 4)
        };
        if start < 64 || start >= end || end > fragment.len() {
            return Err("Invalid RLE segment offsets".into());
        }
        let mut input = start;
        let mut output = 0usize;
        let byte_offset = if cfg!(target_endian = "little") {
            bytes_per_sample - segment - 1
        } else {
            segment
        };
        while input < end {
            // A fragment may have one zero pad byte after its completed segment.
            if output == count && input + 1 == end && fragment[input] == 0 {
                break;
            }
            let control = fragment[input] as i8;
            input += 1;
            if control == -128 {
                continue;
            }
            let run = if control < 0 {
                (1i16 - i16::from(control)) as usize
            } else {
                control as usize + 1
            };
            if run > count - output {
                return Err("RLE packet expands beyond declared pixel count".into());
            }
            let source_bytes = if control < 0 { 1 } else { run };
            if source_bytes > end - input {
                return Err("Truncated RLE packet".into());
            }
            for i in 0..run {
                out[(output + i) * bytes_per_sample + byte_offset] =
                    fragment[input + if control < 0 { 0 } else { i }];
            }
            input += source_bytes;
            output += run;
        }
        if output != count {
            return Err("RLE segment is shorter than declared pixel count".into());
        }
    }
    Ok(out)
}

/// Decode a single monochrome frame. Unsupported clinical transforms fail rather
/// than being ignored. Compressed transfer syntaxes use enabled dicom-rs codecs;
/// unsupported codecs return an error (no external executables or networking).
pub fn decode(path: &Path) -> Result<Frame, String> {
    // Validate metadata BEFORE opening/decoding the potentially compressed pixels.
    let header = read_object(path, false)?;
    let (width, height, count) = dimensions(&header)?;
    let pixel_aspect = pixel_aspect(&header)?;
    let frames = match numbers(&header, "NumberOfFrames")? {
        None => 1.0,
        Some(n) if n.len() == 1 => n[0],
        _ => return Err("Invalid NumberOfFrames".into()),
    };
    if frames != 1.0 {
        return Err("Multiframe DICOM is not supported".into());
    }
    let photo = text(&header, "PhotometricInterpretation");
    if integer(&header, "SamplesPerPixel")? != 1
        || !matches!(photo.as_str(), "MONOCHROME1" | "MONOCHROME2")
    {
        return Err(format!(
            "Color/non-monochrome DICOM is not supported ({photo})"
        ));
    }
    for name in [
        "ModalityLUTSequence",
        "VOILUTSequence",
        "PresentationLUTSequence",
        "SharedFunctionalGroupsSequence",
        "PerFrameFunctionalGroupsSequence",
    ] {
        if header
            .element_by_name_opt(name)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err(format!(
                "{name} is not supported; refusing to ignore its transform"
            ));
        }
    }
    let voi = text(&header, "VOILUTFunction");
    if !voi.is_empty() && voi != "LINEAR" {
        return Err(format!("Unsupported VOI LUT function: {voi}"));
    }
    let presentation = text(&header, "PresentationLUTShape");
    if !presentation.is_empty() && presentation != "IDENTITY" && presentation != "INVERSE" {
        return Err(format!("Unsupported PresentationLUTShape: {presentation}"));
    }
    // MONOCHROME1's standard INVERSE presentation shape expresses the same
    // polarity as its photometric interpretation, not a second inversion.
    if (photo == "MONOCHROME1" && presentation == "IDENTITY")
        || (photo == "MONOCHROME2" && presentation == "INVERSE")
    {
        return Err("PresentationLUTShape conflicts with monochrome polarity".into());
    }
    let allocated = integer(&header, "BitsAllocated")?;
    let stored = integer(&header, "BitsStored")?;
    let high = integer(&header, "HighBit")?;
    let representation = integer(&header, "PixelRepresentation")?;
    if !matches!(allocated, 8 | 16)
        || stored == 0
        || stored > allocated
        || high != stored - 1
        || representation > 1
    {
        return Err(
            "Unsupported/invalid pixel bit layout (requires aligned 8/16-bit integer samples)"
                .into(),
        );
    }
    let scalar = |name: &str| -> Result<Option<f64>, String> {
        match numbers(&header, name)? {
            None => Ok(None),
            Some(v) if v.len() == 1 => Ok(Some(v[0])),
            _ => Err(format!("{name} must contain exactly one value")),
        }
    };
    let (slope, intercept) = match (scalar("RescaleSlope")?, scalar("RescaleIntercept")?) {
        (None, None) => (1.0, 0.0),
        (Some(s), Some(i)) if s != 0.0 => (s, i),
        _ => {
            return Err(
                "RescaleSlope/Intercept must both be present and slope must be nonzero".into(),
            );
        }
    };
    let padding = padding_range(
        scalar("PixelPaddingValue")?,
        scalar("PixelPaddingRangeLimit")?,
        stored,
        representation == 1,
    )?;
    let window = numbers(&header, "WindowWidth")?;
    let center = numbers(&header, "WindowCenter")?;
    let preset = match (window, center) {
        (None, None) => None,
        (Some(w), Some(c)) if w.len() == c.len() && w.iter().all(|w| *w >= 1.0) => {
            Some((w[0], c[0]))
        }
        _ => return Err("Invalid WindowWidth/WindowCenter pair".into()),
    };
    let obj = header;
    let guarded = bounded_compressed_pixels(&obj, width, height, allocated)?;
    let bytes = if let Some(bytes) = guarded {
        bytes
    } else {
        let pixels = obj.decode_pixel_data().map_err(|e| {
            format!("Pixel decoding failed (possibly unsupported compression): {e}")
        })?;
        if pixels.rows() != height
            || pixels.columns() != width
            || pixels.number_of_frames() != 1
            || pixels.samples_per_pixel() != 1
            || u32::from(pixels.bits_allocated()) != allocated
            || u32::from(pixels.bits_stored()) != stored
            || u32::from(pixels.high_bit()) != high
        {
            return Err("Decoded pixel layout disagrees with validated metadata".into());
        }
        pixels.data().to_vec()
    };
    let needed = count * (allocated as usize / 8);
    if bytes.len() != needed && !(needed % 2 == 1 && bytes.len() == needed + 1) {
        return Err(format!(
            "Pixel data length mismatch: expected {needed}, got {}",
            bytes.len()
        ));
    }
    let (values, contrast_samples) = if allocated == 8 {
        rescaled_samples(
            bytes[..needed].iter().map(|b| u16::from(*b)),
            stored,
            representation == 1,
            slope,
            intercept,
            padding,
        )
    } else {
        rescaled_samples(
            bytes[..needed]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_ne_bytes(*b)),
            stored,
            representation == 1,
            slope,
            intercept,
            padding,
        )
    };
    if values.iter().any(|v| !v.is_finite()) {
        return Err("Rescaled pixel values exceed the finite f32 range".into());
    }
    let (window, level) = preset.unwrap_or_else(|| {
        let (window, level) = robust_window(&contrast_samples);
        (f64::from(window), f64::from(level))
    });
    if !(window as f32).is_finite() || !(level as f32).is_finite() {
        return Err("Window/level exceeds the finite f32 range".into());
    }
    Ok(Frame {
        width,
        height,
        pixel_aspect,
        values,
        window: window as f32,
        level: level as f32,
        supplied_window: preset.is_some(),
        contrast_samples,
        inverted: photo == "MONOCHROME1",
    })
}

fn sample_value(raw: u16, bits: u32, signed: bool, slope: f64, intercept: f64) -> f32 {
    (f64::from(stored_value(raw, bits, signed)) * slope + intercept) as f32
}

fn stored_value(raw: u16, bits: u32, signed: bool) -> i32 {
    let mask = (1u32 << bits) - 1;
    let value = u32::from(raw) & mask;
    if signed && value & (1 << (bits - 1)) != 0 {
        value as i32 - (1i32 << bits)
    } else {
        value as i32
    }
}

fn padding_range(
    value: Option<f64>,
    limit: Option<f64>,
    bits: u32,
    signed: bool,
) -> Result<Option<(i32, i32)>, String> {
    let Some(value) = value else {
        return if limit.is_some() {
            Err("PixelPaddingRangeLimit requires PixelPaddingValue".into())
        } else {
            Ok(None)
        };
    };
    let minimum = if signed { -(1i32 << (bits - 1)) } else { 0 };
    let maximum = if signed {
        (1i32 << (bits - 1)) - 1
    } else {
        (1i32 << bits) - 1
    };
    let limit = limit.unwrap_or(value);
    if [value, limit].iter().any(|v| {
        !v.is_finite() || v.fract() != 0.0 || *v < f64::from(minimum) || *v > f64::from(maximum)
    }) {
        return Err("Pixel padding must contain integers within the stored pixel range".into());
    }
    Ok(Some((value.min(limit) as i32, value.max(limit) as i32)))
}

fn rescaled_samples(
    raw: impl Iterator<Item = u16> + Clone,
    bits: u32,
    signed: bool,
    slope: f64,
    intercept: f64,
    padding: Option<(i32, i32)>,
) -> (Vec<f32>, Vec<f32>) {
    let valid = |value| padding.is_none_or(|(low, high)| value < low || value > high);
    let count = raw
        .clone()
        .filter(|v| valid(stored_value(*v, bits, signed)))
        .count();
    let sample_count = count.min(8192);
    let mut samples = Vec::with_capacity(sample_count);
    let mut valid_index = 0;
    let values = raw
        .map(|raw| {
            let stored = stored_value(raw, bits, signed);
            let value = sample_value(raw, bits, signed, slope, intercept);
            if valid(stored) {
                // Midpoints of equally sized strata in the VALID population, not the first N
                // pixels or a stride which could repeatedly hit a padding border.
                if samples.len() < sample_count
                    && valid_index == (2 * samples.len() + 1) * count / (2 * sample_count)
                {
                    samples.push(value);
                }
                valid_index += 1;
            }
            value
        })
        .collect();
    (values, samples)
}

/// A deterministic 1–99% heuristic, not a calibrated diagnostic VOI recommendation.
/// Width/center follow DICOM LINEAR's half-unit conventions. All-padding images get a
/// finite neutral window; valid zeroes are retained like every other valid intensity.
pub fn robust_window(samples: &[f32]) -> (f32, f32) {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable_by(f32::total_cmp);
    if sorted.is_empty() {
        return (1.0, 0.5);
    }
    let clip = sorted.len() / 100;
    let low = f64::from(sorted[clip]);
    let high = f64::from(sorted[sorted.len() - 1 - clip]);
    (
        ((high - low + 1.0).max(1.0).min(f64::from(f32::MAX))) as f32,
        ((low + high + 1.0) / 2.0) as f32,
    )
}

pub fn representative_indices(count: usize) -> Vec<usize> {
    let size = count.min(5);
    (0..size)
        .map(|i| {
            if size == 1 {
                0
            } else {
                i * (count - 1) / (size - 1)
            }
        })
        .collect()
}

/// Standard DICOM LINEAR window, including the width=1 threshold case.
/// `invert` is an additional user toggle, XORed with MONOCHROME1 polarity.
/// Invalid arguments/frame shapes return an empty buffer, never a misleading image.
#[cfg(test)]
pub fn window_rgba(frame: &Frame, window: f32, level: f32, invert: bool) -> Vec<u8> {
    let Some(count) = (frame.width as usize).checked_mul(frame.height as usize) else {
        return Vec::new();
    };
    if count == 0
        || count > MAX_PIXELS
        || count != frame.values.len()
        || !window.is_finite()
        || window < 1.0
        || !level.is_finite()
        || frame.values.iter().any(|v| !v.is_finite())
    {
        return Vec::new();
    }
    let mut rgba = Vec::with_capacity(count * 4);
    let center = f64::from(level) - 0.5;
    let width = f64::from(window) - 1.0;
    for &value in &frame.values {
        let value = f64::from(value);
        let gray = if value <= center - width / 2.0 {
            0
        } else if value > center + width / 2.0 {
            255
        } else {
            (((value - center) / width + 0.5) * 255.0).round() as u8
        };
        let gray = if frame.inverted ^ invert {
            255 - gray
        } else {
            gray
        };
        rgba.extend_from_slice(&[gray, gray, gray, 255]);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_window_clips_extremes_but_preserves_zero_and_samples_the_whole_valid_population() {
        let raw = (0..20_000u16).collect::<Vec<_>>();
        let (_, samples) = rescaled_samples(raw.iter().copied(), 16, false, 1.0, 0.0, None);
        assert_eq!(samples.len(), 8192);
        assert!(samples[0] < 3.0 && samples[8191] > 19_990.0);
        let mut raw = vec![65535; 20_000];
        raw.extend((0..20_000).map(|i| (i % 1000) as u16));
        raw[30_000] = 60_000;
        let (values, samples) = rescaled_samples(
            raw.iter().copied(),
            16,
            false,
            1.0,
            0.0,
            Some((65535, 65535)),
        );
        assert_eq!(values[0], 65535.0, "padding pixels are not rewritten");
        assert!(values.contains(&60_000.0));
        assert!(samples.contains(&0.0), "valid zero is not padding");
        assert_eq!(samples.len(), 8192);
        let (width, level) = robust_window(&samples);
        assert!((900.0..1000.0).contains(&width), "{width}");
        assert!((450.0..550.0).contains(&level), "{level}");
        assert_eq!(robust_window(&[]), (1.0, 0.5));
        assert_eq!(robust_window(&[42.0; 100]), (1.0, 42.5));
        let (_, empty) =
            rescaled_samples([0u16; 10].into_iter(), 16, false, 1.0, 0.0, Some((0, 0)));
        assert!(empty.is_empty());
        assert_eq!(robust_window(&empty), (1.0, 0.5));
        assert!(robust_window(&[-f32::MAX, f32::MAX]).0.is_finite());
    }

    #[test]
    fn signed_padding_range_is_excluded_before_negative_rescale_and_metadata_is_validated() {
        let padding = padding_range(Some(-1.0), Some(-2048.0), 12, true).unwrap();
        let (values, samples) = rescaled_samples(
            [0x0800, 0x0fff, 0, 0x07ff].into_iter(),
            12,
            true,
            -2.0,
            10.0,
            padding,
        );
        assert_eq!(values, [4106.0, 12.0, 10.0, -4084.0]);
        assert_eq!(samples, [10.0, -4084.0]);
        assert_eq!(robust_window(&samples), (4095.0, -2036.5));
        for (value, limit) in [
            (None, Some(0.0)),
            (Some(1.5), None),
            (Some(2048.0), None),
            (Some(-2049.0), None),
            (Some(0.0), Some(f64::NAN)),
        ] {
            assert!(padding_range(value, limit, 12, true).is_err());
        }
        assert_eq!(padding_range(None, None, 12, true).unwrap(), None);
        assert_eq!(
            padding_range(Some(0.0), None, 16, false).unwrap(),
            Some((0, 0))
        );
    }

    #[test]
    fn series_sampling_is_unique_deterministic_and_bounded_to_five_slices() {
        assert!(representative_indices(0).is_empty());
        assert_eq!(representative_indices(1), [0]);
        assert_eq!(representative_indices(3), [0, 1, 2]);
        assert_eq!(representative_indices(100), [0, 24, 49, 74, 99]);
        assert_eq!(
            representative_indices(100_000),
            [0, 24_999, 49_999, 74_999, 99_999]
        );
    }

    fn frame(values: &[f32]) -> Frame {
        Frame {
            width: values.len() as u32,
            height: 1,
            pixel_aspect: 1.0,
            values: values.to_vec(),
            window: 4.0,
            level: 2.0,
            supplied_window: true,
            contrast_samples: values.to_vec(),
            inverted: false,
        }
    }

    #[test]
    fn dicom_linear_window_boundaries_and_width_one() {
        let f = frame(&[-1.0, 0.0, 1.5, 3.0, 4.0]);
        let rgba = window_rgba(&f, 4.0, 2.0, false);
        assert_eq!(
            rgba.as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[0])
                .collect::<Vec<_>>(),
            [0, 0, 128, 255, 255]
        );
        let rgba = window_rgba(&frame(&[0.49, 0.5, 0.51]), 1.0, 1.0, false);
        assert_eq!(
            rgba.as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[0])
                .collect::<Vec<_>>(),
            [0, 0, 255]
        );
        assert!(rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
    }

    #[test]
    fn monochrome_one_and_user_inversion_are_independent() {
        let mut f = frame(&[0.0, 3.0]);
        f.inverted = true;
        assert_eq!(
            window_rgba(&f, 4.0, 2.0, false),
            [255, 255, 255, 255, 0, 0, 0, 255]
        );
        assert_eq!(
            window_rgba(&f, 4.0, 2.0, true),
            [0, 0, 0, 255, 255, 255, 255, 255]
        );
    }

    #[test]
    fn signed_stored_bits_padding_and_modality_rescale() {
        assert_eq!(sample_value(0xafff, 12, true, 2.0, -1024.0), -1026.0);
        assert_eq!(sample_value(0x0800, 12, true, 1.0, 0.0), -2048.0);
        assert_eq!(sample_value(0xffff, 16, false, 0.5, -10.0), 32757.5);
        assert_eq!(sample_value(0xff, 8, true, 1.0, 0.0), -1.0);
    }

    fn instance(name: &str, number: i32, x: f64) -> Instance {
        Instance {
            path: name.into(),
            number: Some(number),
            position: Some([x, 0.0, 0.0]),
            orientation: Some([0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
        }
    }

    #[test]
    fn spatial_sort_uses_orientation_normal_not_z_or_instance_number() {
        let mut instances = vec![instance("b", 1, 20.0), instance("a", 2, 10.0)];
        sort_instances(&mut instances);
        assert_eq!(instances[0].path, PathBuf::from("a"));
    }

    #[test]
    fn inconsistent_or_missing_geometry_falls_back_for_whole_series() {
        for missing in [false, true] {
            let mut instances = vec![instance("a", 2, 10.0), instance("b", 1, 20.0)];
            instances[0].orientation = if missing {
                None
            } else {
                Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0])
            };
            sort_instances(&mut instances);
            assert_eq!(instances[0].path, PathBuf::from("b"));
        }
    }

    #[test]
    fn invalid_window_or_frame_is_not_rendered() {
        assert!(window_rgba(&frame(&[0.0]), 0.0, 0.0, false).is_empty());
        assert!(window_rgba(&frame(&[f32::NAN]), 1.0, 0.0, false).is_empty());
        let mut f = frame(&[0.0]);
        f.width = 2;
        assert!(window_rgba(&f, 1.0, 0.0, false).is_empty());
    }

    // Generate a tiny Explicit VR Little Endian Part 10 file independently of
    // the decoder. Keeping the fixture here avoids binary fixtures/dev dependencies.
    fn generated_dicom(study: &str, series: &str, instance: u32, x: f64) -> Vec<u8> {
        generated_dicom_with_syntax(study, series, instance, x, "1.2.840.10008.1.2.1")
    }

    fn generated_dicom_with_syntax(
        study: &str,
        series: &str,
        instance: u32,
        x: f64,
        syntax: &str,
    ) -> Vec<u8> {
        fn element(out: &mut Vec<u8>, group: u16, tag: u16, vr: &[u8; 2], value: &[u8]) {
            let mut value = value.to_vec();
            if value.len() % 2 == 1 {
                value.push(if vr == b"UI" { 0 } else { b' ' });
            }
            out.extend_from_slice(&group.to_le_bytes());
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(vr);
            if matches!(vr, b"OB" | b"OW") {
                out.extend_from_slice(&[0, 0]);
                out.extend_from_slice(&(value.len() as u32).to_le_bytes());
            } else {
                out.extend_from_slice(&(value.len() as u16).to_le_bytes());
            }
            out.extend_from_slice(&value);
        }
        let sop_class = b"1.2.840.10008.5.1.4.1.1.2";
        let sop_instance = format!("{study}.{series}.{instance}");
        let mut meta = Vec::new();
        element(&mut meta, 2, 1, b"OB", &[0, 1]);
        element(&mut meta, 2, 2, b"UI", sop_class);
        element(&mut meta, 2, 3, b"UI", sop_instance.as_bytes());
        element(&mut meta, 2, 0x10, b"UI", syntax.as_bytes());
        element(&mut meta, 2, 0x12, b"UI", b"2.25.123456789");
        let mut out = vec![0; 128];
        out.extend_from_slice(b"DICM");
        element(&mut out, 2, 0, b"UL", &(meta.len() as u32).to_le_bytes());
        out.extend_from_slice(&meta);
        element(&mut out, 8, 0x16, b"UI", sop_class);
        element(&mut out, 8, 0x18, b"UI", sop_instance.as_bytes());
        element(&mut out, 8, 0x60, b"CS", b"CT");
        element(&mut out, 0x10, 0x10, b"PN", b"TEST-PATIENT");
        element(&mut out, 0x10, 0x20, b"LO", b"TEST-ID");
        element(&mut out, 0x10, 0x21, b"LO", b"TEST-ISSUER");
        element(&mut out, 0x20, 0x0d, b"UI", study.as_bytes());
        element(&mut out, 0x20, 0x0e, b"UI", series.as_bytes());
        element(&mut out, 0x20, 0x13, b"IS", instance.to_string().as_bytes());
        element(&mut out, 0x20, 0x32, b"DS", format!("{x}\\0\\0").as_bytes());
        element(&mut out, 0x20, 0x37, b"DS", b"0\\1\\0\\0\\0\\1");
        element(&mut out, 0x28, 2, b"US", &1u16.to_le_bytes());
        element(&mut out, 0x28, 4, b"CS", b"MONOCHROME2");
        element(&mut out, 0x28, 0x10, b"US", &1u16.to_le_bytes());
        element(&mut out, 0x28, 0x11, b"US", &4u16.to_le_bytes());
        element(&mut out, 0x28, 0x30, b"DS", b"2\\0.5");
        for (tag, value) in [(0x100, 16u16), (0x101, 12), (0x102, 11), (0x103, 1)] {
            element(&mut out, 0x28, tag, b"US", &value.to_le_bytes());
        }
        element(&mut out, 0x28, 0x1050, b"DS", b"-1024.5");
        element(&mut out, 0x28, 0x1051, b"DS", b"8191");
        element(&mut out, 0x28, 0x1052, b"DS", b"-1024");
        element(&mut out, 0x28, 0x1053, b"DS", b"2");
        let pixels: Vec<u8> = [0x0800u16, 0x0fff, 0, 0x07ff]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        element(&mut out, 0x7fe0, 0x10, b"OW", &pixels);
        out
    }

    #[test]
    fn tiny_file_declaring_four_gib_element_is_rejected_before_eager_parse() {
        let mut bytes = generated_dicom("1.2.3", "1.2.4", 1, 0.0);
        let start = bytes.len() - 20;
        // Replace PixelData with a private UN primitive, so BOTH header indexing
        // and full decode must validate it (rather than stop before PixelData).
        bytes[start..start + 8].copy_from_slice(&[0x77, 0x77, 0x10, 0, b'U', b'N', 0, 0]);
        bytes[start + 8..start + 12].copy_from_slice(&0xffff_fffeu32.to_le_bytes());
        assert!(bytes.len() < 1024);
        for header_only in [true, false] {
            let error = preflight_file(&bytes, header_only).unwrap_err();
            assert!(error.contains("length exceeds allocation limit"), "{error}");
        }
    }

    #[test]
    fn header_preflight_does_not_stop_at_nested_pixel_data() {
        // A sequence item may itself contain PixelData. The object reader's
        // read_until boundary is TOP-LEVEL, so nested pixels must not bypass the
        // allocation guard for subsequent nested attributes.
        let bytes = [
            8, 0, 0x10, 0x11, b'S', b'Q', 0, 0, 255, 255, 255, 255, 0xfe, 0xff, 0, 0xe0, 255, 255,
            255, 255, 0xe0, 0x7f, 0x10, 0, b'O', b'W', 0, 0, 2, 0, 0, 0, 0, 0, 0x77, 0x77, 0x10, 0,
            b'U', b'N', 0, 0, 254, 255, 255, 255,
        ];
        let syntax = TransferSyntaxRegistry.get("1.2.840.10008.1.2.1").unwrap();
        let error = preflight_dataset(&bytes, syntax, true).unwrap_err();
        assert!(error.contains("length exceeds allocation limit"), "{error}");
    }

    #[test]
    fn dataset_and_frame_deflate_are_rejected_before_decompression() {
        for uid in ["1.2.840.10008.1.2.1.99", "1.2.840.10008.1.2.8.1"] {
            let bytes = generated_dicom_with_syntax("1.2.3", "1.2.4", 1, 0.0, uid);
            for header_only in [true, false] {
                let error = preflight_file(&bytes, header_only).unwrap_err();
                assert!(error.contains("bounded loader"), "{error}");
            }
        }
    }

    #[test]
    fn embedded_jpeg_dimensions_cannot_override_dicom_bounds() {
        // SOI + baseline SOF, with one grayscale component and enormous dimensions.
        let jpeg = [
            0xff, 0xd8, 0xff, 0xc0, 0, 11, 8, 0xfd, 0xe8, 0xfd, 0xe8, 1, 1, 0x11, 0,
        ];
        let error = preflight_jpeg(&jpeg, 4, 1, 8).unwrap_err();
        assert!(error.contains("disagree"), "{error}");
    }

    fn generated_rle_fragment() -> Vec<u8> {
        let mut fragment = vec![0; 64];
        fragment[..4].copy_from_slice(&2u32.to_le_bytes());
        fragment[4..8].copy_from_slice(&64u32.to_le_bytes());
        fragment[8..12].copy_from_slice(&69u32.to_le_bytes());
        fragment.extend_from_slice(&[3, 8, 15, 0, 7]); // Four most significant bytes.
        fragment.extend_from_slice(&[3, 0, 255, 0, 255]); // Four least significant bytes.
        fragment
    }

    #[test]
    fn rle_packets_are_bounded_before_expansion_and_preserve_sample_bytes() {
        let mut fragment = generated_rle_fragment();
        let decoded = decode_rle(&fragment, 4, 16).unwrap();
        let words: Vec<_> = decoded
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_ne_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(words, [0x0800, 0x0fff, 0, 0x07ff]);
        fragment[64] = 129; // Repeat 128 bytes, despite a declared four pixels.
        assert!(
            decode_rle(&fragment, 4, 16)
                .unwrap_err()
                .contains("beyond declared")
        );
    }

    #[test]
    fn generated_signed_dicom_decodes_rescale_once_and_indexes_by_study_and_series() {
        struct TestFolder(PathBuf);
        impl Drop for TestFolder {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let folder =
            std::env::temp_dir().join(format!("fastdicom-test-{}-{nonce}", std::process::id()));
        fs::create_dir(&folder).unwrap();
        let folder = TestFolder(folder);
        fs::create_dir(folder.0.join("nested")).unwrap();
        let first = folder.0.join("first.dcm");
        let second = folder.0.join("nested/second.dcm");
        fs::write(&first, generated_dicom("1.2.3", "1.2.4", 2, 10.0)).unwrap();
        fs::write(&second, generated_dicom("1.2.3", "1.2.4", 1, 20.0)).unwrap();
        fs::write(
            folder.0.join("other-study"),
            generated_dicom("1.2.5", "1.2.4", 1, 10.0),
        )
        .unwrap();
        fs::write(
            folder.0.join("other-series"),
            generated_dicom("1.2.3", "1.2.6", 1, 10.0),
        )
        .unwrap();
        fs::write(
            folder.0.join("DICOMDIR"),
            b"index placeholder: scan containing folder",
        )
        .unwrap();
        fs::write(folder.0.join("notes.txt"), b"not DICOM").unwrap();

        let f = decode(&first).unwrap();
        assert_eq!((f.width, f.height), (4, 1));
        assert_eq!(f.pixel_aspect, 4.0);
        assert_eq!(f.values, [-5120.0, -1026.0, -1024.0, 3070.0]);
        assert_eq!((f.window, f.level, f.inverted), (8191.0, -1024.5, false));
        assert!(f.supplied_window);
        assert_eq!(f.contrast_samples, f.values);
        // Exercise actual DICOM padding metadata and an absent supplied VOI pair.
        let mut automatic = generated_dicom("1.2.3", "1.2.7", 1, 0.0);
        for tag in [0x1050u16, 0x1051] {
            let prefix = [0x28, 0, tag as u8, (tag >> 8) as u8, b'D', b'S'];
            let offset = automatic.windows(6).position(|v| v == prefix).unwrap();
            let len = u16::from_le_bytes([automatic[offset + 6], automatic[offset + 7]]) as usize;
            automatic.drain(offset..offset + 8 + len);
        }
        let offset = automatic
            .windows(6)
            .position(|v| v == [0x28, 0, 0x52, 0x10, b'D', b'S'])
            .unwrap();
        automatic.splice(
            offset..offset,
            [0x28, 0, 0x20, 1, b'S', b'S', 2, 0, 0, 0xf8],
        ); // -2048, stored domain.
        let auto_path = folder.0.join("automatic.dcm");
        fs::write(&auto_path, &automatic).unwrap();
        let auto = decode(&auto_path).unwrap();
        assert!(!auto.supplied_window);
        assert_eq!(auto.values, f.values);
        assert_eq!(auto.contrast_samples, [-1026.0, -1024.0, 3070.0]);
        assert_eq!((auto.window, auto.level), (4097.0, 1022.5));
        // Keep the original indexing assertions focused on the original study fixtures.
        fs::remove_file(auto_path).unwrap();
        let rgba = window_rgba(&f, f.window, f.level, false);
        assert_eq!(
            rgba.as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[0])
                .collect::<Vec<_>>(),
            [0, 127, 128, 255]
        );

        let scan = scan_folder(&folder.0.join("DICOMDIR")).unwrap();
        assert_eq!(scan.skipped, 2);
        assert_eq!(scan.series.len(), 3);
        let series = scan.series.iter().find(|s| s.id == "1.2.3/1.2.4").unwrap();
        assert_eq!(series.files, [first, second]);
        assert_eq!(series.modality, "CT");
        assert_eq!(series.patient_id, "TEST-ID");
        assert_eq!(series.patient_issuer, "TEST-ISSUER");
        // Even a malformed export reusing both UIDs must not combine two patients' slices.
        let mut conflicting = generated_dicom("1.2.3", "1.2.4", 2, 10.0);
        let offset = conflicting
            .windows(7)
            .position(|w| w == b"TEST-ID")
            .unwrap();
        conflicting[offset] = b'X';
        fs::write(folder.0.join("first.dcm"), conflicting).unwrap();
        let guarded = scan_folder(&folder.0).unwrap();
        assert_eq!(guarded.skipped, 3);
        assert_eq!(
            guarded
                .series
                .iter()
                .find(|s| s.id == "1.2.3/1.2.4")
                .unwrap()
                .files
                .len(),
            1
        );

        // Exercise the retained compressed path through the same public decode
        // boundary, including lazy encapsulated-item validation and rescale.
        let mut rle = generated_dicom_with_syntax("1.2.3", "1.2.4", 3, 30.0, "1.2.840.10008.1.2.5");
        rle.truncate(rle.len() - 20);
        rle.extend_from_slice(&[0xe0, 0x7f, 0x10, 0, b'O', b'W', 0, 0, 255, 255, 255, 255]);
        rle.extend_from_slice(&[0xfe, 0xff, 0, 0xe0, 0, 0, 0, 0]); // Empty basic offset table.
        let fragment = generated_rle_fragment();
        rle.extend_from_slice(&[0xfe, 0xff, 0, 0xe0]);
        rle.extend_from_slice(&(fragment.len() as u32).to_le_bytes());
        rle.extend_from_slice(&fragment);
        rle.extend_from_slice(&[0xfe, 0xff, 0xdd, 0xe0, 0, 0, 0, 0]);
        let rle_path = folder.0.join("signed-rle.dcm");
        fs::write(&rle_path, rle).unwrap();
        let compressed = decode(&rle_path).unwrap();
        assert_eq!(compressed.values, f.values);
        assert_eq!(compressed.pixel_aspect, f.pixel_aspect);
    }

    #[test]
    fn pixel_aspect_uses_spacing_fallbacks_and_rejects_invalid_geometry() {
        let bytes = generated_dicom("1.2.3", "1.2.4", 1, 0.0);
        let offset = bytes
            .windows(6)
            .position(|w| w == [0x28, 0, 0x30, 0, b'D', b'S'])
            .unwrap();
        let parse_aspect = |bytes: &[u8]| {
            let obj = OpenFileOptions::new()
                .from_reader(Cursor::new(bytes))
                .unwrap();
            pixel_aspect(&obj)
        };
        for (group, tag) in [
            (0x28u16, 0x30u16),
            (0x18, 0x1164),
            (0x18, 0x2010),
            (0x28, 0x34),
        ] {
            let mut fixture = bytes.clone();
            fixture[offset..offset + 2].copy_from_slice(&group.to_le_bytes());
            fixture[offset + 2..offset + 4].copy_from_slice(&tag.to_le_bytes());
            if tag == 0x34 {
                fixture[offset + 4..offset + 6].copy_from_slice(b"IS");
                fixture[offset + 8..offset + 14].copy_from_slice(b"4\\1   ");
            }
            assert_eq!(parse_aspect(&fixture).unwrap(), 4.0);
        }
        for invalid in [b"0\\0.5 ", b"-1\\0.5", b"2     ", b"NaN\\1 "] {
            let mut fixture = bytes.clone();
            fixture[offset + 8..offset + 14].copy_from_slice(invalid);
            assert!(parse_aspect(&fixture).is_err());
        }
        let mut missing = bytes.clone();
        missing.drain(offset..offset + 14);
        assert_eq!(parse_aspect(&missing).unwrap(), 1.0);
    }
}
