FastDICOM demo sample pack
=========================
REAL PUBLIC DE-IDENTIFIED IMAGES - DEMONSTRATION ONLY, NOT FOR CLINICAL USE.
Names and birth dates are fictional; the anatomy is not synthetic.

Open this whole folder in FastDICOM: 3 studies, 4 series, 102 images.

- CT-Chest: 50 contiguous chest CT slices, instances 42-91 of a 133-image
  LIDC-IDRI series. This is an excerpt, not a complete CT examination.
- MRI-Brain/T1-Postcontrast: all 20 slices of the source T1 postcontrast series.
- MRI-Brain/T2: 30 contiguous slices, instances 4-33 of 36 source images.
  These real MRI sequences cover the internal auditory canal/posterior fossa,
  NOT the whole brain. Source authors already defaced the MRI images.
- XRay-Chest: the existing real PA and lateral chest radiographs (2 images).

Every image uses PatientName=Doe^John and PatientBirthDate=19700101
(1 January 1970). These values do not describe the source participants.
PatientAge was removed and a demo notice added to PatientComments.
Source patient pseudonyms, UIDs, sex, geometry and rescale remain; source
dates are de-identification values, not an actual clinical chronology.
CT also omits non-patient name placeholders and private collection tags.
Pixel data was not changed during this demo preparation.

CT, MRI and radiographs represent DIFFERENT source participants:
LIDC-IDRI-0001, VS-SEG-243 and LIDC-IDRI-0232 respectively. Do not treat them
as one John Doe clinical history. Only the two MRI series share a study and
can be compared together; the viewer clears other patients from the other pane.

Sources and licenses
--------------------
CT and radiographs: TCIA LIDC-IDRI, CC BY 3.0.
https://doi.org/10.7937/K9/TCIA.2015.LO9QL9SX
https://www.cancerimagingarchive.net/collection/lidc-idri/
See LICENSE-TCIA-LIDC-IDRI.txt.

MRI: TCIA Vestibular-Schwannoma-MC-RC, Version 2, CC BY 4.0.
https://doi.org/10.7937/hrzh-2n82
https://www.cancerimagingarchive.net/collection/vestibular-schwannoma-mc-rc/
See LICENSE-TCIA-VS-MC-RC.txt.

Retrieved directly through TCIA's NBIA API on 2026-11-07. Full dataset
citations and exact series download URLs are in CITATION.txt.
Retain attribution, licenses, changes notices and the TCIA usage-policy link
when redistributing, and require downstream users to do likewise:
https://www.cancerimagingarchive.net/data-usage-policies-and-restrictions/
Do not attempt to identify or contact participants. Public de-identification
and our metadata/visual checks are not a guarantee of anonymity.

The authors acknowledge the National Cancer Institute and the Foundation
for the National Institutes of Health, and their critical role in the
creation of the free publicly available LIDC/IDRI Database used in this study.
