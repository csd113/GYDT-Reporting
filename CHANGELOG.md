# Release notes

## 1.0.1

- Require a separately supplied GYDT Profile for the actual desktop workflow.
  The downloads include no private catalogue, site definitions or draft
  templates. New local state is separate from earlier app stores.
- Retain the existing format-1 ZIP and JPEG export-copy behavior. Exports use
  the configured site's exact tokens and the approved catalogue digest.

## 1.0.0 · demo release withdrawn

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
