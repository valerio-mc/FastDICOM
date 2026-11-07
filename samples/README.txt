FastDICOM demo sample pack
=========================
DEMONSTRATION ONLY - NOT FOR DIAGNOSIS OR TREATMENT.
CT/MRI images are synthetic mathematical phantoms, not anatomically
accurate clinical studies. Chest X-rays are REAL public de-identified
radiographs from TCIA LIDC-IDRI, not synthetic images.

Open this folder using FastDICOM's Open folder command (Cmd+O).
The app discovers the DICOM images recursively.

Included (3 studies, 4 series, 82 images):
- XRay-Chest: real PA and lateral chest radiographs (2 images, 1 series).
- MRI-Brain/T1: 20 axial T1 phantom images, 256 x 256.
- MRI-Brain/T2: 20 axial T2 phantom images, 256 x 256.
- CT-Torso: 40 axial chest/torso phantom images, 512 x 512.
  This is NOT a dedicated abdominal CT study.

The T1 and T2 series belong to the same synthetic patient and can be
compared side by side. CT, MR and X-ray have distinct sample patient IDs;
FastDICOM intentionally clears the other pane when switching patients.

Fictional demographics in EVERY image:
- PatientName (0010,0010): Doe^John (standard DICOM family^given encoding).
- PatientBirthDate (0010,0030): 19700101 (1 January 1970).
These demographics do not describe the real radiograph subject.
The viewer displays the encoded patient name and birth date. Privacy mode
hides both, together with study descriptions and dates. Source dates are
demo/de-identification values, not a clinical history; source sex and public
dataset identifiers remain unchanged.

Changes: replaced PatientName/PatientBirthDate, removed any original
PatientAge to avoid conflicting demographics, and added a demo notice in
PatientComments. Pixel data and study/series/instance identifiers were
preserved. The original synthetic chest X-ray was replaced by the real pair.
The unsupported RGB CT dose-report image remains omitted.
All included images use single-frame MONOCHROME2, 16-bit integer storage,
and supported uncompressed little-endian transfer syntaxes.
README/LICENSE are non-image files and may count as skipped on import.

CT/MRI source: CharruaSoft DICOM Samples (CC0 1.0)
https://www.charruasoft.com/dicom-samples
https://www.charruasoft.com/products/samples/zip/charruasoft-dicom-samples-ct.zip
https://www.charruasoft.com/products/samples/zip/charruasoft-dicom-samples-mr.zip
Retrieved: 2026-11-07
See LICENSE.txt for the CharruaSoft license ONLY (includes the source
catalog's ECG notice; no ECG files are included here).

Chest X-ray source: TCIA LIDC-IDRI (CC BY 3.0; NOT covered by CC0)
https://www.cancerimagingarchive.net/collection/lidc-idri/
https://doi.org/10.7937/K9/TCIA.2015.LO9QL9SX
https://creativecommons.org/licenses/by/3.0/
Original public dataset ID: LIDC-IDRI-0232
Study UID: 1.3.6.1.4.1.14519.5.2.1.6279.6001.329528275573582416795597707236
Series UID: 1.3.6.1.4.1.14519.5.2.1.6279.6001.210618824080932755350370549788
Retrieved directly through TCIA's NBIA API: 2026-11-07.
Full dataset citation: see CITATION.txt.
See LICENSE-TCIA-LIDC-IDRI.txt for the source license and usage policy.
Retain attribution, license links, source/change notices, and pass along
TCIA's data usage policy when redistributing. Do not attempt to identify
or contact participants. Policy:
https://www.cancerimagingarchive.net/data-usage-policies-and-restrictions/

The authors acknowledge the National Cancer Institute and the Foundation
for the National Institutes of Health, and their critical role in the
creation of the free publicly available LIDC/IDRI Database used in this study.

Open this folder for the multi-study examples. The built-in Synthetic demo
remains a separate mathematical phantom.
