use super::*;

/// Standard DICOM LINEAR window, including the width=1 threshold case.
/// `invert` is an additional user toggle, XORed with MONOCHROME1 polarity.
/// Independent reference for valid frames and windows.
pub(crate) fn window_rgba(frame: &Frame, window: f32, level: f32, invert: bool) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(frame.values.len() * 4);
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
    let (_, empty) = rescaled_samples([0u16; 10].into_iter(), 16, false, 1.0, 0.0, Some((0, 0)));
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
    for (raw, bits, signed, slope, intercept, expected) in [
        (0xafff, 12, true, 2.0, -1024.0, -1026.0),
        (0x0800, 12, true, 1.0, 0.0, -2048.0),
        (0xffff, 16, false, 0.5, -10.0, 32757.5),
        (0xff, 8, true, 1.0, 0.0, -1.0),
    ] {
        let (values, _) =
            rescaled_samples(std::iter::once(raw), bits, signed, slope, intercept, None);
        assert_eq!(values, [expected]);
    }
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
        8, 0, 0x10, 0x11, b'S', b'Q', 0, 0, 255, 255, 255, 255, 0xfe, 0xff, 0, 0xe0, 255, 255, 255,
        255, 0xe0, 0x7f, 0x10, 0, b'O', b'W', 0, 0, 2, 0, 0, 0, 0, 0, 0x77, 0x77, 0x10, 0, b'U',
        b'N', 0, 0, 254, 255, 255, 255,
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
