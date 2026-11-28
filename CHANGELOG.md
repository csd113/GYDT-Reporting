# Release notes

## 1.0.5

### Desktop app

- The Windows download contains one executable. Opening the webcam preview
  installs its bundled helper in local app data and checks it before launch.
  An existing helper beside the app is not used or removed.
- With webcam capture enabled, the photo box and the new-sample **Take Image**
  button open the live preview directly. Browse or drop a JPG instead, or skip
  the prompt. Cancelling a preview keeps any existing photo.
- Choose and save a named camera in Settings. If it disconnects, reconnect and
  retry or select another camera. The app never substitutes another device.
- Manual JPG attachment remains available if webcam capture is disabled or
  the bundled helper cannot start. The macOS workflow is unchanged.

### WordPress importer

- New OPUS imports add the original `.0` file to the existing **Spectra File**
  field on the Test Result. The file follows the site’s normal media access
  rules, just like a manual upload. Older private spectra remain available to
  authorized staff from their original Results.

## 1.0.4

### Desktop app

- Choose today’s export folder at the start of each Vancouver work day. The
  selection is remembered for the rest of that day. Yesterday’s folder is not
  reused unless you choose it again.
- Exports stage temporary files in local app data. Choose an export folder on
  the same drive as the app data. The app doesn’t clean up old destination files.
- OPUS export is off by default. When enabled, the app uses your saved OPUS
  source folder to copy the exact submission’s `.0` file beside and inside the
  ZIP. If access is denied, unload the file in OPUS and retry. Nothing from that
  export is saved in the destination or recorded as exported.
- JPEGs export as `.jpg` without changing the image bytes. A photo already
  saved under that name is not overwritten.
- Windows webcam capture is off by default. When enabled, choose a color camera
  by name and explicitly start its live preview. Capture, retake, or attach the
  photo as a JPEG. Where supported, switch cameras or refresh and reconnect
  without reopening the app. The app never switches cameras on its own.
- Additional samples deliberately linked to the same visit show which original
  tab holds their contact details. Select the indicator to go to that tab.
  Contact drafts use the original tab’s details without copying them to each sample.
- The local text draft shows the intended recipient on a `To:` line. Email and
  text drafts omit the sample heading for one report and keep numbered headings
  for multiple reports. The messages still omit internal submission numbers.

### WordPress importer

- Use production importer v1.0.4 with desktop v1.0.4 for ZIPs that include OPUS.
  Staff with access to edit the corresponding Test Result can download its
  spectrum. It is not added to the public Media Library.
- Today’s Uploads shows successful uploads across staff accounts, with live
  Draft or Published status and the highest numeric sample suffix first. The
  site-day count includes each successful upload once, before personal hiding.
  Remove and Clear List only hide rows for the account using them.
- Removed the Newest and Oldest controls. Check ZIP is now at the bottom of
  the page and still checks without importing.
- Replace an older installed importer with the v1.0.4 ZIP without deleting it.
  On WordPress 5.8 and newer, future verified releases can appear under
  Installed Plugins → Update now. Automatic updates are not enabled.

### Maintenance

- Retired the obsolete demo and its importer package. Production package and
  desktop regression checks still use purpose-made synthetic data.
- Require matching macOS, Windows, and production importer downloads and their
  SHA-256 checksums for every release. Branch rollover waits until all three
  packages have been verified after publication.

## 1.0.3

- Put exact substance names first in both pickers while keeping dash-insensitive search.
- Show multiple results and a scroll cue in compact windows.
- Keep the picker open when text is pasted and reset the list to the first match.
- The WordPress importer is unchanged.

## 1.0.2

- Build the approved profile into the app, so it opens without a separate folder.
- Keep the same local store, catalogue IDs, site mapping, and ZIP format for
  installations that used the approved v1.0.1 profile.
- The production WordPress importer now timestamps new drafts with the
  exporting computer's time, displayed in Vancouver local time. Collection
  and analysis dates remain separate.

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
