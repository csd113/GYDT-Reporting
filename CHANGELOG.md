# Release notes

## 1.0.0 · preparation

- Client email and text draft bodies omit the internal submission number. It
  remains in local records, internal ZIP exports and local draft filenames.
- Windows starts with viewport dimensions 25% smaller and fits the monitor
  work area. The app uses the selected Indigo Orbit icon.
- JPEGs with camera metadata can be attached. The export copy removes metadata
  the existing importer does not accept without re-encoding the image or
  changing the source photo bytes.
- Windows x64 and Apple Silicon package validation use only the reviewed
  synthetic catalogue and draft templates. CI builds packages for validation
  without uploading them.
- The separate local-only WordPress fixture importer now uses the normal
  WordPress draft and Featured Image lifecycle. It is not the staff plugin and
  is not intended for a real site.
