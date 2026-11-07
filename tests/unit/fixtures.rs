//! Deterministic pixels for compositor/presentation tests, never a user-facing demo.

/// Exercise normal series navigation/requests without decoding external files.
pub(crate) fn cached_study(count: usize) -> crate::State {
    let mut state = crate::State::default();
    let files: Vec<_> = (0..count)
        .map(|i| std::path::PathBuf::from(format!("fixture-slice-{i}.dcm")))
        .collect();
    let frame = std::sync::Arc::new(crate::dicom::Frame {
        width: 2,
        height: 2,
        values: vec![-1000.0, 0.0, 50.0, 1000.0],
        contrast_samples: vec![-1000.0, 0.0, 50.0, 1000.0],
        window: 1500.0,
        level: -500.0,
        supplied_window: true,
        inverted: false,
        pixel_aspect: 1.0,
    });
    for path in &files {
        state
            .cache
            .lock()
            .unwrap()
            .insert(path.clone(), frame.clone());
    }
    state.series.push(crate::dicom::Series {
        id: "fixture-study/series".into(),
        study_uid: "fixture-study".into(),
        study_date: None,
        series_number: None,
        description: "Presentation fixture".into(),
        modality: "MR".into(),
        study_description: "Test study".into(),
        patient_name: "Test patient".into(),
        patient_birth_date: None,
        patient_id: "fixture-patient".into(),
        patient_issuer: "".into(),
        files,
    });
    state.selected = Some(0);
    state
}

pub(crate) fn synthetic_frame(index: usize) -> crate::dicom::Frame {
    let size = 512;
    let z = (index as f32 - 80.0) / 100.0;
    let mut values = vec![-1000.0; size * size];
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 - 256.0) / 200.0;
            let v = (y as f32 - 256.0) / 200.0;
            let r = u * u / 0.85 + v * v;
            let noise = ((x * 17 + y * 31 + index * 7) % 23) as f32 - 11.0;
            let mut value = -1000.0;
            if r < 0.91 - z * z * 0.15 {
                value = 25.0 + noise;
            }
            if r > 0.82 - z * z * 0.15 && r < 0.91 - z * z * 0.15 {
                value = -90.0 + noise;
            }
            for side in [-1.0, 1.0] {
                let lung = (u - side * 0.37).powi(2) / 0.08 + (v + 0.12).powi(2) / 0.27;
                if lung < 1.0 && r < 0.8 {
                    value = -780.0 + noise * 4.0;
                }
                let bone = (u - side * 0.68).powi(2) + (v - 0.08).powi(2);
                if bone < 0.006 {
                    value = 850.0 + noise;
                }
            }
            if u * u / 0.035 + (v - 0.51).powi(2) / 0.025 < 1.0 {
                value = 650.0 + noise;
            }
            if (u - 0.08).powi(2) / 0.09 + (v - 0.10).powi(2) / 0.12 < 1.0 {
                value = 70.0 + noise;
            }
            values[y * size + x] = value;
        }
    }
    crate::dicom::Frame {
        width: size as u32,
        height: size as u32,
        contrast_samples: values.iter().step_by(32).copied().collect(),
        values,
        window: 1500.0,
        level: -500.0,
        supplied_window: true,
        inverted: false,
        pixel_aspect: 1.0,
    }
}
