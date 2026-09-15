# Downloads & Brave

A familiar Location tab, with Linux filesystem semantics.

## Set a standard folder location

Right-click Downloads or Documents → Properties → Location. Choose an existing writable directory, check the destination, and confirm before applying. The previous user-directory setting is backed up. No existing files are moved, merged or deleted.

## Use a stable mount for a network destination

Use a persistent Linux mount path such as /mnt/nas/downloads. Temporary per-login GVfs paths and an unmounted SMB URL are not suitable replacements for a standard folder.

> An offline NAS cannot accept a download. The search index is not an offline synchronization engine.

## Optionally sync native Brave profiles

Also update Brave’s download directory opens a separate profile-selection and confirmation step. Fully quit Brave, including background processes, first. Only download and Save As directory fields are changed, with private backups and field-level restoration.

## Sandboxed or custom browser profiles

For unsupported Flatpak/Snap, custom or policy-managed profiles, set the same mounted path manually in Brave. Filesystem access permissions may also need review.

```sh
brave://settings/downloads
```

## Review the optional mount helper

The optional helper prepares persistent systemd mount/automount configuration. It needs administrator approval and uses a root-readable plaintext CIFS credential file, separate from the desktop keyring. It is not invoked during package installation. See desktop/ZORIN-SETUP.md before using it.

---

OpenXplorer 1.0.2. Project-authored documentation: AGPL-3.0-only.
