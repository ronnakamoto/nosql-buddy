import type { ReactNode } from "react";

/**
 * SVG icon set for NoSQLBuddy.
 *
 * All icons share a 16x16 viewBox, 1.5px stroke, currentColor.
 * No external dependencies; icons are inline SVG paths.
 */

type IconProps = {
  size?: number;
  className?: string;
  style?: React.CSSProperties;
};

function makeIcon(path: ReactNode, displayName: string) {
  const Icon = ({ size = 16, className, style }: IconProps) => (
    <svg
      width={size}
      height={size}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      style={{ flexShrink: 0, ...style }}
      aria-hidden="true"
    >
      {path}
    </svg>
  );
  Icon.displayName = displayName;
  return Icon;
}

export const IconSearch = makeIcon(
  <>
    <circle cx="7" cy="7" r="4.5" />
    <path d="M10.5 10.5L14 14" />
  </>,
  "IconSearch",
);

export const IconTerminal = makeIcon(
  <>
    <path d="M2 4h12v8H2z" />
    <path d="M4.5 7l2 1.5L4.5 10" />
    <path d="M8.5 10h3" />
  </>,
  "IconTerminal",
);

export const IconShieldCheck = makeIcon(
  <>
    <path d="M8 1.5L3 3.5v4c0 3 2 5.5 5 7 3-1.5 5-4 5-7v-4L8 1.5z" />
    <path d="M5.5 8l1.8 1.8L10.5 6.5" />
  </>,
  "IconShieldCheck",
);

export const IconDatabase = makeIcon(
  <>
    <ellipse cx="8" cy="3.5" rx="5" ry="2" />
    <path d="M3 3.5v9c0 1.1 2.2 2 5 2s5-.9 5-2v-9" />
    <path d="M3 8c0 1.1 2.2 2 5 2s5-.9 5-2" />
  </>,
  "IconDatabase",
);

export const IconLayers = makeIcon(
  <>
    <path d="M8 2L14 5L8 8L2 5L8 2z" />
    <path d="M2 8l6 3l6-3" />
    <path d="M2 11l6 3l6-3" />
  </>,
  "IconLayers",
);

export const IconGrid = makeIcon(
  <>
    <rect x="2" y="2" width="5" height="5" rx="0.5" />
    <rect x="9" y="2" width="5" height="5" rx="0.5" />
    <rect x="2" y="9" width="5" height="5" rx="0.5" />
    <rect x="9" y="9" width="5" height="5" rx="0.5" />
  </>,
  "IconGrid",
);

export const IconServer = makeIcon(
  <>
    <rect x="2" y="2.5" width="12" height="4" rx="1" />
    <rect x="2" y="9.5" width="12" height="4" rx="1" />
    <circle cx="4.5" cy="4.5" r="0.5" fill="currentColor" />
    <circle cx="4.5" cy="11.5" r="0.5" fill="currentColor" />
  </>,
  "IconServer",
);

export const IconBeaker = makeIcon(
  <>
    <path d="M6 2v4L2.5 12c-.5 1 .2 2 1.3 2h8.4c1.1 0 1.8-1 1.3-2L10 6V2" />
    <path d="M5 2h6" />
    <path d="M4 10h8" />
  </>,
  "IconBeaker",
);

export const IconCheckCircle = makeIcon(
  <>
    <circle cx="8" cy="8" r="6" />
    <path d="M5 8l2 2L11 6" />
  </>,
  "IconCheckCircle",
);

export const IconCircleDash = makeIcon(
  <>
    <path d="M8 2a6 6 0 1 0 0 12a6 6 0 0 0 0-12z" strokeDasharray="2.5 2" />
  </>,
  "IconCircleDash",
);

export const IconChevronRight = makeIcon(
  <>
    <path d="M6 3l5 5l-5 5" />
  </>,
  "IconChevronRight",
);

export const IconClose = makeIcon(
  <>
    <path d="M4 4l8 8M12 4l-8 8" />
  </>,
  "IconClose",
);

export const IconPlus = makeIcon(
  <>
    <path d="M8 3v10M3 8h10" />
  </>,
  "IconPlus",
);
