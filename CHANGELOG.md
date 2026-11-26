# Release notes

## 1.0.0 · preparation

- Client email and text draft bodies omit the internal submission number. It
  remains in local records, internal ZIP exports and local draft filenames.
- Windows starts with viewport dimensions 25% smaller and fits the monitor
  work area. The app uses the selected Indigo Orbit icon.
- JPEGs with camera metadata can be attached. The export copy removes metadata
  the existing importer does not accept without re-encoding the image or
  changing the source photo bytes.
- Public desktop packages use the reviewed synthetic catalogue and draft
  templates, show the Demo name, and use their own local data folder.
  Inherited staff data-directory settings are ignored.
- The separate local-only WordPress fixture importer now uses the normal
  WordPress draft and Featured Image lifecycle. It is not the staff plugin and
  is not intended for a real site.
