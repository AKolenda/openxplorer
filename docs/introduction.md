# Introduction

A familiar way to explore. A different kind of ownership.

## Meet OpenXplorer

OpenXplorer is a Windows File Explorer-inspired file manager for Linux, built for Zorin OS and intended to extend to compatible Ubuntu and Debian installations. It brings a familiar tabbed interface to your local folders and SMB shares.

Previously called Winspace, the project is now published as OpenXplorer. The desktop application and this website are distributed under AGPL-3.0-only; original third-party notices remain intact.

![OpenXplorer’s real HTML interface browsing Projects on a sample NAS](../apps/web/public/assets/screenshots/explorer-light.png)

*Actual HTML interface. Sample files; no live NAS connection.*

## Your files. Your network. Your workflow.

Navigate with clickable breadcrumbs, pin folders, type to select a filename, resize columns, and use either a classic or compact context menu. Search opted-in filename indexes, browse ZIP contents read-only, and inspect existing exposed snapshots.

The interactive preview runs the same HTML, CSS, icons and controls as the desktop application, with a simulated storage adapter. It cannot access your computer, NAS, browser settings or keyring. On a desktop-sized screen, use the preview controls to open a fictional sample NAS, switch appearance, or replay a pinning walkthrough. The interactive explorer is intentionally not loaded on phones; the documentation and screenshots remain available.

- Local files and SMB shares in one interface.
- A searchable Settings page, with explicit controls for desktop integration.
- Source included alongside the installer; no account needed to use the app.

[Open the interactive application preview](../apps/web/public/app-preview.html) — sample files, no access to your computer.

## Native storage. Local interface.

Since 2.0.0 the desktop app is a native GTK 4 application written in Rust. GIO/GVfs provides the filesystem, SMB and phone layer; SQLite stores the optional filename index. The Python/WebKitGTK app of 1.x is deprecated and no longer released. This Next.js website is separate and is not required to run the app; its interactive preview still renders the 1.x HTML interface, which the native app reproduces.

## Know what you are installing

Version 2.0.0 replaces the Python app with the native GTK 4 app under the same name, settings and saved passwords, and adds Fluent icons, a categorised Settings page, undo, and Flatpak and distribution packages. Zorin is the primary target; Ubuntu 24.04+, Debian 13, Fedora, openSUSE and Arch are supported through their packages, and every other distribution through the Flatpak. The known gaps are listed in the changelog.

> Use a disposable folder and a non-critical share first. SMB servers other than the maintainer's, phones and USB drives have not yet been accepted on real hardware with the native app.

## Start with one folder

Read Installation, open your home directory, and test a network share. Enable indexing and default-file-manager integration only after checking basic file operations.

---

OpenXplorer 2.0.0. Project-authored documentation: AGPL-3.0-only.
