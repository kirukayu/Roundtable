/**
 * One icon set, 24x24, 1.75 stroke, rounded caps. Consistency matters more than
 * variety, so everything here is drawn to the same rules.
 */

interface IconProps {
  size?: number;
  className?: string;
  style?: React.CSSProperties;
}

function svg(path: React.ReactNode, { size = 18, className, style }: IconProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      style={style}
      aria-hidden="true"
      focusable="false"
    >
      {path}
    </svg>
  );
}

export const Icon = {
  Library: (p: IconProps) =>
    svg(
      <>
        <rect x="3" y="3" width="7" height="18" rx="1.6" />
        <rect x="14" y="3" width="7" height="10" rx="1.6" />
        <path d="M14 17h7" />
      </>,
      p,
    ),
  Grid: (p: IconProps) =>
    svg(
      <>
        <rect x="3" y="3" width="7.5" height="7.5" rx="1.6" />
        <rect x="13.5" y="3" width="7.5" height="7.5" rx="1.6" />
        <rect x="3" y="13.5" width="7.5" height="7.5" rx="1.6" />
        <rect x="13.5" y="13.5" width="7.5" height="7.5" rx="1.6" />
      </>,
      p,
    ),
  Play: (p: IconProps) => svg(<path d="M7 4.8 19 12 7 19.2Z" fill="currentColor" />, p),
  Layers: (p: IconProps) =>
    svg(
      <>
        <path d="m12 3 9 4.8-9 4.8-9-4.8Z" />
        <path d="m3 12.6 9 4.8 9-4.8" />
        <path d="m3 17.4 9 4.8 9-4.8" />
      </>,
      p,
    ),
  Save: (p: IconProps) =>
    svg(
      <>
        <path d="M5 3h11l3 3v15H5Z" />
        <path d="M9 3v6h6V3" />
        <rect x="8" y="13" width="8" height="8" rx="1" />
      </>,
      p,
    ),
  Users: (p: IconProps) =>
    svg(
      <>
        <circle cx="9" cy="8" r="3.4" />
        <path d="M3 20c0-3.4 2.7-5.6 6-5.6s6 2.2 6 5.6" />
        <path d="M16.5 5.4a3.4 3.4 0 0 1 0 5.6" />
        <path d="M18 14.8c2 .8 3 2.6 3 5.2" />
      </>,
      p,
    ),
  Download: (p: IconProps) =>
    svg(
      <>
        <path d="M12 3v11.5" />
        <path d="m7.5 10.5 4.5 4.5 4.5-4.5" />
        <path d="M4 19.5h16" />
      </>,
      p,
    ),
  Settings: (p: IconProps) =>
    svg(
      <>
        <circle cx="12" cy="12" r="3.2" />
        <path d="M19.4 14.4a1.6 1.6 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.6 1.6 0 0 0-1.8-.3 1.6 1.6 0 0 0-1 1.5v.2a2 2 0 1 1-4 0v-.1a1.6 1.6 0 0 0-1-1.5 1.6 1.6 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.6 1.6 0 0 0 .3-1.8 1.6 1.6 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.6 1.6 0 0 0 1.5-1 1.6 1.6 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.6 1.6 0 0 0 1.8.3H9a1.6 1.6 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.6 1.6 0 0 0 1 1.5 1.6 1.6 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.6 1.6 0 0 0-.3 1.8V9a1.6 1.6 0 0 0 1.5 1h.2a2 2 0 1 1 0 4h-.1a1.6 1.6 0 0 0-1.5 1Z" />
      </>,
      p,
    ),
  Tools: (p: IconProps) =>
    svg(
      <>
        <path d="M14.6 3.6a4.6 4.6 0 0 0 6 6L14 16.2 8.8 21.4a2.4 2.4 0 0 1-3.4-3.4L10.6 12Z" />
        <path d="m5 5 3.2 3.2" />
      </>,
      p,
    ),
  Search: (p: IconProps) =>
    svg(
      <>
        <circle cx="10.8" cy="10.8" r="6.8" />
        <path d="m15.8 15.8 4.4 4.4" />
      </>,
      p,
    ),
  Plus: (p: IconProps) => svg(<path d="M12 5v14M5 12h14" />, p),
  Close: (p: IconProps) => svg(<path d="M6 6l12 12M18 6 6 18" />, p),
  Minus: (p: IconProps) => svg(<path d="M5 12h14" />, p),
  Square: (p: IconProps) => svg(<rect x="5" y="5" width="14" height="14" rx="2" />, p),
  Check: (p: IconProps) => svg(<path d="m5 12.6 4.6 4.6L19 7" />, p),
  Chevron: (p: IconProps) => svg(<path d="m9 5 7 7-7 7" />, p),
  Back: (p: IconProps) => svg(<path d="M19 12H5m0 0 6-6m-6 6 6 6" />, p),
  Folder: (p: IconProps) => svg(<path d="M3 6.5A1.5 1.5 0 0 1 4.5 5h4.2l2 2.6h8.8A1.5 1.5 0 0 1 21 9.1v9.4a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 3 18.5Z" />, p),
  Trash: (p: IconProps) =>
    svg(
      <>
        <path d="M4 7h16M9.5 7V4.5h5V7" />
        <path d="M6.5 7v12.5a1.5 1.5 0 0 0 1.5 1.5h8a1.5 1.5 0 0 0 1.5-1.5V7" />
        <path d="M10 11.5v5M14 11.5v5" />
      </>,
      p,
    ),
  Copy: (p: IconProps) =>
    svg(
      <>
        <rect x="9" y="9" width="12" height="12" rx="2" />
        <path d="M15 6V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h1" />
      </>,
      p,
    ),
  Refresh: (p: IconProps) =>
    svg(
      <>
        <path d="M20.5 12a8.5 8.5 0 1 1-2.7-6.2" />
        <path d="M20.5 4v5h-5" />
      </>,
      p,
    ),
  Shield: (p: IconProps) => svg(<path d="M12 3 4.8 5.8v6c0 4.5 3 7.9 7.2 9 4.2-1.1 7.2-4.5 7.2-9v-6Z" />, p),
  ShieldOff: (p: IconProps) =>
    svg(
      <>
        <path d="M12 3 4.8 5.8v6c0 4.5 3 7.9 7.2 9 4.2-1.1 7.2-4.5 7.2-9v-6Z" />
        <path d="m4 4 16 16" />
      </>,
      p,
    ),
  Warning: (p: IconProps) =>
    svg(
      <>
        <path d="M10.3 4.3 2.6 18a2 2 0 0 0 1.7 3h15.4a2 2 0 0 0 1.7-3L13.7 4.3a2 2 0 0 0-3.4 0Z" />
        <path d="M12 9.5v4M12 17.2v.1" />
      </>,
      p,
    ),
  Info: (p: IconProps) =>
    svg(
      <>
        <circle cx="12" cy="12" r="9" />
        <path d="M12 11.2v5M12 7.8v.1" />
      </>,
      p,
    ),
  Swap: (p: IconProps) =>
    svg(
      <>
        <path d="M4 8h13m0 0-4-4m4 4-4 4" />
        <path d="M20 16H7m0 0 4-4m-4 4 4 4" />
      </>,
      p,
    ),
  Broom: (p: IconProps) =>
    svg(
      <>
        <path d="m14.5 3.5 6 6" />
        <path d="m17 6-8.5 8.5" />
        <path d="M8.5 14.5 12 18l-3.2 2.2a1.6 1.6 0 0 1-2-.2l-2.8-2.8a1.6 1.6 0 0 1-.2-2Z" />
      </>,
      p,
    ),
  Key: (p: IconProps) =>
    svg(
      <>
        <circle cx="8" cy="8" r="4.6" />
        <path d="m11.4 11.4 8.6 8.6M16.4 16.4l2-2M19 19l2-2" />
      </>,
      p,
    ),
  Merge: (p: IconProps) =>
    svg(
      <>
        <path d="M5 3v5a4 4 0 0 0 4 4h10" />
        <path d="M5 21v-6" />
        <path d="m15 8 4 4-4 4" />
      </>,
      p,
    ),
  Star: (p: IconProps) =>
    svg(
      <path d="m12 3.6 2.6 5.4 5.9.8-4.3 4.1 1 5.9-5.2-2.8-5.2 2.8 1-5.9L3.5 9.8l5.9-.8Z" />,
      p,
    ),
  StarFilled: (p: IconProps) =>
    svg(
      <path
        d="m12 3.6 2.6 5.4 5.9.8-4.3 4.1 1 5.9-5.2-2.8-5.2 2.8 1-5.9L3.5 9.8l5.9-.8Z"
        fill="currentColor"
      />,
      p,
    ),
  Clock: (p: IconProps) =>
    svg(
      <>
        <circle cx="12" cy="12" r="9" />
        <path d="M12 7v5.4l3.4 2" />
      </>,
      p,
    ),
  Steam: (p: IconProps) =>
    svg(
      <>
        <circle cx="12" cy="12" r="9" />
        <circle cx="15.2" cy="9.2" r="2.7" />
        <circle cx="8.5" cy="14.8" r="2.2" />
        <path d="m10.5 13.6 2.4-1.8" />
      </>,
      p,
    ),
  Dots: (p: IconProps) =>
    svg(
      <>
        <circle cx="12" cy="5" r="1.4" fill="currentColor" />
        <circle cx="12" cy="12" r="1.4" fill="currentColor" />
        <circle cx="12" cy="19" r="1.4" fill="currentColor" />
      </>,
      p,
    ),
  Pin: (p: IconProps) =>
    svg(
      <>
        <path d="M9 3h6l-.8 6 3.3 3.3H6.5L9.8 9Z" />
        <path d="M12 12.3V21" />
      </>,
      p,
    ),
  Bolt: (p: IconProps) => svg(<path d="M13.5 2 4 13.5h6.5L10 22l9.5-11.5H13Z" />, p),
  Sound: (p: IconProps) =>
    svg(
      <>
        <path d="M11 5 6.5 9H3v6h3.5L11 19Z" />
        <path d="M15.5 9.2a4 4 0 0 1 0 5.6" />
        <path d="M18.2 6.4a8 8 0 0 1 0 11.2" />
      </>,
      p,
    ),
  Muted: (p: IconProps) =>
    svg(
      <>
        <path d="M11 5 6.5 9H3v6h3.5L11 19Z" />
        <path d="m16 10 5 4M21 10l-5 4" />
      </>,
      p,
    ),
  Box: (p: IconProps) =>
    svg(
      <>
        <path d="m12 2.8 8.5 4.4v9.6L12 21.2 3.5 16.8V7.2Z" />
        <path d="M3.7 7.3 12 11.6l8.3-4.3M12 11.6V21" />
      </>,
      p,
    ),
};
