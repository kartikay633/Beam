// Line icons in the Windows 11 style: 20×20, 1.4 px strokes in currentColor.

import type { ReactNode } from "react";

function Icon({ children, size = 20, stroke = 1.4 }: { children: ReactNode; size?: number; stroke?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth={stroke}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

type P = { size?: number };

/** The Beam logo: two points joined by a beam, on the accent square. */
export const BeamLogo = ({ size = 18 }: P) => (
  <svg width={size} height={size} viewBox="0 0 100 100" aria-hidden="true">
    <rect className="logo-bg" width="100" height="100" rx="24" />
    <g fill="#FFFFFF">
      <circle cx="30" cy="50" r="12" />
      <circle cx="70" cy="50" r="12" />
      <rect x="30" y="44" width="40" height="12" />
    </g>
  </svg>
);

export const ShareIcon = (p: P) => (
  <Icon {...p}>
    <path d="M10 13V3M6.5 6.5 10 3l3.5 3.5M4 11.5v3A2.5 2.5 0 0 0 6.5 17h7a2.5 2.5 0 0 0 2.5-2.5v-3" />
  </Icon>
);
export const ClockIcon = (p: P) => (
  <Icon {...p}>
    <circle cx="10" cy="10" r="7" />
    <path d="M10 6.5V10l2.5 1.8" />
  </Icon>
);
export const PhoneIcon = (p: P) => (
  <Icon {...p}>
    <rect x="6" y="2.5" width="8" height="15" rx="2" />
    <path d="M9 14.5h2" />
  </Icon>
);
export const LaptopIcon = (p: P) => (
  <Icon {...p}>
    <rect x="3.5" y="4.5" width="13" height="9" rx="1.2" />
    <path d="M2 16.5h16" />
  </Icon>
);
export const DeviceIcon = (p: P) => (
  <Icon {...p}>
    <rect x="4" y="4" width="12" height="12" rx="2.5" />
    <circle cx="10" cy="10" r="2" />
  </Icon>
);
export const GearIcon = (p: P) => (
  <Icon {...p}>
    <circle cx="10" cy="10" r="2.6" />
    <path d="M8.6 2.6h2.8l.5 2 1.8 1 2-.6 1.4 2.4-1.5 1.5v2.2l1.5 1.5-1.4 2.4-2-.6-1.8 1-.5 2H8.6l-.5-2-1.8-1-2 .6-1.4-2.4 1.5-1.5V8.9L2.9 7.4l1.4-2.4 2 .6 1.8-1z" />
  </Icon>
);
export const EyeIcon = (p: P) => (
  <Icon {...p}>
    <path d="M2 10s3-5.5 8-5.5S18 10 18 10s-3 5.5-8 5.5S2 10 2 10z" />
    <circle cx="10" cy="10" r="2.3" />
  </Icon>
);
export const EyeOffIcon = (p: P) => (
  <Icon {...p}>
    <path d="M3 3l14 14M8.2 4.8A7.6 7.6 0 0 1 10 4.5c5 0 8 5.5 8 5.5a14 14 0 0 1-2.2 2.9M5.4 6.3A13.7 13.7 0 0 0 2 10s3 5.5 8 5.5a7.3 7.3 0 0 0 3.5-.9" />
  </Icon>
);
export const ShieldIcon = (p: P) => (
  <Icon {...p}>
    <path d="M10 2.5 4 5v5c0 3.5 2.5 6.2 6 7.5 3.5-1.3 6-4 6-7.5V5z" />
    <path d="m7.5 10 1.8 1.8 3.3-3.3" />
  </Icon>
);
export const QrIcon = (p: P) => (
  <Icon {...p}>
    <rect x="3" y="3" width="5" height="5" rx="1" />
    <rect x="12" y="3" width="5" height="5" rx="1" />
    <rect x="3" y="12" width="5" height="5" rx="1" />
    <path d="M12 12h2v2h-2zM15 15h2v2h-2zM15 12h2M12 15v2" />
  </Icon>
);
export const FolderIcon = (p: P) => (
  <Icon {...p}>
    <path d="M2.5 6a1.5 1.5 0 0 1 1.5-1.5h3.5l1.8 1.8H16a1.5 1.5 0 0 1 1.5 1.5v6.7A1.5 1.5 0 0 1 16 16H4a1.5 1.5 0 0 1-1.5-1.5z" />
  </Icon>
);
export const ArrowRightIcon = (p: P) => (
  <Icon {...p}>
    <path d="M4 10h12M11 5l5 5-5 5" />
  </Icon>
);
export const ChevronDownIcon = (p: P) => (
  <Icon {...p}>
    <path d="M5 7.5 10 12.5l5-5" />
  </Icon>
);
export const CheckIcon = (p: P) => (
  <Icon {...p} stroke={1.8}>
    <path d="M4.5 10.5 8 14l7.5-8" />
  </Icon>
);
export const CloseIcon = (p: P) => (
  <Icon {...p}>
    <path d="M5 5l10 10M15 5 5 15" />
  </Icon>
);
export const BluetoothIcon = (p: P) => (
  <Icon {...p} stroke={1.6}>
    <path d="m6 6 8 8-4 4V2l4 4-8 8" />
  </Icon>
);
export const InfoIcon = (p: P) => (
  <Icon {...p}>
    <circle cx="10" cy="10" r="7.5" />
    <path d="M10 9v5M10 6.3v.2" />
  </Icon>
);
export const ErrorIcon = (p: P) => (
  <Icon {...p}>
    <circle cx="10" cy="10" r="7.5" />
    <path d="M10 6v5M10 13.7v.2" />
  </Icon>
);
export const MoreIcon = ({ size = 16 }: P) => (
  <svg width={size} height={size} viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
    <circle cx="3" cy="8" r="1.2" />
    <circle cx="8" cy="8" r="1.2" />
    <circle cx="13" cy="8" r="1.2" />
  </svg>
);
export const PlusIcon = (p: P) => (
  <Icon {...p}>
    <path d="M10 4v12M4 10h12" />
  </Icon>
);
export const ArrowUpIcon = (p: P) => (
  <Icon {...p}>
    <path d="M10 16V4M5 9l5-5 5 5" />
  </Icon>
);
export const ArrowDownIcon = (p: P) => (
  <Icon {...p}>
    <path d="M10 4v12M5 11l5 5 5-5" />
  </Icon>
);

/** A coloured file-type glyph, as in File Explorer. */
export function FileGlyph({ name, size = 28 }: { name: string; size?: number }) {
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  if (["mp4", "mov", "mkv", "avi", "webm", "3gp"].includes(ext)) {
    return (
      <svg width={size} height={size} viewBox="0 0 28 28" aria-hidden="true">
        <rect x="3" y="5" width="22" height="18" rx="3" className="glyph-video-bg" />
        <path d="M12 10.5v7l6-3.5z" className="glyph-video" />
      </svg>
    );
  }
  if (["jpg", "jpeg", "png", "heic", "gif", "webp", "bmp"].includes(ext)) {
    return (
      <svg width={size} height={size} viewBox="0 0 28 28" aria-hidden="true">
        <rect x="3" y="4" width="22" height="20" rx="3" className="glyph-image-bg" />
        <path d="m6 20 6-7 4 4 3-3 3 6z" className="glyph-image" />
      </svg>
    );
  }
  return (
    <svg width={size} height={size} viewBox="0 0 28 28" aria-hidden="true">
      <rect x="6" y="3" width="16" height="22" rx="2.5" className="glyph-doc-bg" />
      <path d="M10 11h8M10 14.5h8M10 18h5" className="glyph-doc" strokeWidth="1.4" strokeLinecap="round" fill="none" />
    </svg>
  );
}
