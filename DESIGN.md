---
version: alpha
name: Beam
description: A quiet desktop utility for nearby file sharing.
colors:
  primary: "#14171c"
  background: "#f4f5f7"
  surface: "#ffffff"
  secondaryText: "#5f6672"
  border: "#e4e6ea"
  success: "#1f7a4d"
  danger: "#b42318"
typography:
  sans:
    fontFamily: '"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif'
    fontSize: "14px"
    lineHeight: "1.45"
  mono:
    fontFamily: "ui-monospace, monospace"
rounded:
  DEFAULT: "8px"
  lg: "12px"
spacing:
  desktopPage: "32px"
  narrowPage: "18px"
components:
  button:
    rounded: "8px"
  card:
    rounded: "12px"
---

# Beam design context

## Overview

Beam is a product utility for people sharing files between Android and Windows. The repository's visual decision is recorded in docs/DECISIONS.md (29 October 2026): neutral greys, one ink accent, hairline borders, familiar controls, and no decorative gradients. Preserve that identity during repairs. Nearby devices are a plain list: one row per device with its name, how it's reached and a Send button; clicking a row or dropping files on it sends. Controls and transfer history remain quiet and legible.

The interface is English. It follows the operating system's light/dark preference. This file records the established design; **desktop/ui/src/styles.css is the canonical runtime token source**, including dark-theme values. Android's theme remains owned by its Compose theme file. This repair does not generate or duplicate a second token package.

## Colors

Use the shared CSS variables for surfaces, text, borders, hover, focus, and semantic states. Green means completed/success, red means failure or removing trust, and amber means a recoverable caution. State must also have text. Dark mode reverses the ink accent while retaining semantic roles. Forced-colors mode uses system colors and visible borders.

## Typography

Use the system Segoe UI stack for familiar Windows rendering without network font downloads. Names use 500–600 weight; captions and byte counts are smaller. Fingerprints use monospace. Truncated filenames expose their full text through a title and remain visible in the incoming-offer file list.

## Layout

Share, Activity, Your devices, and Settings use one navigation shell. The sidebar is 190px on desktop and 155px below 780px. Below 560px it becomes a horizontal navigation row. The main panel owns scrolling, with stable scrollbar space. Peer buttons, file picking, visibility, transfer cancel, pairing, and settings remain accessible without drag gestures.

## Elevation & Depth

Use surfaces and borders for structure. Shadows are reserved for menus. Do not imply an operating-system Mica backdrop unless the native window actually supplies one.

## Shapes

Controls use an 8px radius and larger panels 12px. Devices use circular icon containers. Preserve the shared border and focus variables.

## Components

The canonical behavior owners are in UX-CONTRACT.md. Native selects are intentional: their popup geometry and keyboard interaction belong to Windows. Buttons retain their dimensions when disabled/busy. Menus close with Escape. Pairing failures have a visible retry. Removing a device uses an inline confirmation naming the device and consequence. Progress uses actual byte counts; indeterminate motion is reserved for setup and reconnecting. All motion respects reduced-motion preferences.

## Do's and Don'ts

Explain missing visibility, denied permissions, and connection failures in plain language. Say “no shared Wi-Fi needed”, because Wi-Fi is still used. Keep Bluetooth's setup role distinct from file transport. Demo-mode devices and throughput are illustrative and never evidence of a physical transfer. Do not reset settings drafts when unrelated transfer snapshots arrive.
