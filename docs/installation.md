# Installation

Install the stable release without replacing your desktop.

## Requirements

A compatible Debian-family system with Python 3.10+, GTK 3, WebKitGTK 4.1 (2.40+), GIO/GVfs and a graphical user session. The .deb declares these runtime dependencies; APT resolves them. Zorin is the primary target, not the only possible host.

There are no Node.js or pnpm requirements for the desktop application.

## Install OpenXplorer

Close every old Winspace/OpenXplorer window and finish file operations. Download the .deb, then run these commands from that download directory as your normal user.

```sh
sudo apt update
sudo apt install ./openxplorer_1.1.1_all.deb
openxplorer --check
openxplorer
```

> This is a local unsigned package, not an APT repository. Installation does not change your default file manager, Downloads folder, browser preferences, or network mounts.

## Update from inside the app

Starting with 1.1.0, click Check for updates at the bottom of the sidebar. This contacts the public OpenXplorer GitHub Releases API only when requested. Review the version and release notes, then choose Install update. Finish file operations and tab moves first.

The installed Debian app downloads the release installer, verifies its GitHub SHA-256 digest and package identity, then asks for system administrator approval through polkit. APT installs the update without removing packages. Choose Restart now when finished; restarting closes existing windows and tabs.

Older releases need one manual upgrade to 1.1.0 before this control is available. Source checkouts can check versions but cannot install in-app. Updates are not live code patches; the running app must restart. GitHub HTTPS and asset digests are the trust boundary, not an independent publisher signature.

## Upgrading from Winspace

The new package replaces winspace-explorer versions older than 0.8.0. Existing settings, caches, desktop IDs and credential identifiers are deliberately retained. The winspace and winspace-mount-share commands are compatibility aliases.

Stop the previous background reveal service before upgrading: winspace --quit. Do not force-kill a process that is transferring files. Re-pin the installed OpenXplorer launcher if the dock has cached the old display name.

## First-run checklist

Open a disposable folder. Create and rename a test file, then send it to Trash. Connect to a non-critical SMB share before trusting the app with your main storage. For a phone, unlock it and select file transfer or trust the computer when prompted; a GVfs-supported device then appears under This PC. See Known limitations in the project README before enabling advanced operations.

## Blank window or graphics trouble

Close the application completely and try the software-rendering launch. This changes this launch only, not your desktop graphics configuration.

```sh
openxplorer --quit
openxplorer --software-rendering
```

## Remove the application

In Settings, restore previous default handlers and disable Show in folder integration before removal. Unmount/remove persistent mounts separately after closing files. Removing the application does not undo custom folder locations or delete your files.

```sh
sudo apt remove openxplorer
```

## The Software card before installation

A downloaded .deb can have a generic card before installation even though its installed card shows the folder icon, application name and AGPL license. The embedded catalog is not necessarily available to Software until installed and indexed. Another icon or homepage URL cannot guarantee the local pre-install view.

A signed repository with a pre-indexed AppStream catalog can supply that information before installation. This project does not configure or deploy such a repository as part of installing the local package.

---

OpenXplorer 1.1.1. Project-authored documentation: AGPL-3.0-only.
