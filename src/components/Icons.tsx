interface IconProps {
  size?: number;
  className?: string;
}

function svg(
  path: React.ReactNode,
  { size = 14, className }: IconProps,
  box = 16,
) {
  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${box} ${box}`}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={className}
      aria-hidden="true"
    >
      {path}
    </svg>
  );
}

export const ChevronRight = (p: IconProps) => svg(<path d="M6 3.5 10.5 8 6 12.5" />, p);
export const ChevronDown = (p: IconProps) => svg(<path d="M3.5 6 8 10.5 12.5 6" />, p);

export const Search = (p: IconProps) =>
  svg(
    <>
      <circle cx="7" cy="7" r="4.25" />
      <path d="m10.2 10.2 3 3" />
    </>,
    p,
  );

export const Plus = (p: IconProps) => svg(<path d="M8 3.5v9M3.5 8h9" />, p);

export const Close = (p: IconProps) => svg(<path d="M4 4l8 8M12 4l-8 8" />, p);

export const Dots = (p: IconProps) =>
  svg(
    <>
      <circle cx="8" cy="3.5" r="1.1" fill="currentColor" stroke="none" />
      <circle cx="8" cy="8" r="1.1" fill="currentColor" stroke="none" />
      <circle cx="8" cy="12.5" r="1.1" fill="currentColor" stroke="none" />
    </>,
    p,
  );

export const Folder = (p: IconProps) =>
  svg(<path d="M1.8 12.5v-9h4l1.4 1.7h7v7.3z" />, p);

export const FolderOpen = (p: IconProps) =>
  svg(
    <>
      <path d="M1.8 12.5v-9h4l1.4 1.7h7v1.6" />
      <path d="M1.8 12.5 3.9 6.8h11.3l-2.1 5.7z" />
    </>,
    p,
  );

export const Collection = (p: IconProps) =>
  svg(
    <>
      <rect x="2.2" y="2.5" width="11.6" height="11" rx="1.6" />
      <path d="M5.4 2.5v11" />
    </>,
    p,
  );

export const Doc = (p: IconProps) =>
  svg(
    <>
      <path d="M4 2.2h5l3 3v8.6H4z" />
      <path d="M9 2.2v3.2h3" />
    </>,
    p,
  );

export const Eye = (p: IconProps) =>
  svg(
    <>
      <path d="M1.6 8s2.4-4.2 6.4-4.2S14.4 8 14.4 8s-2.4 4.2-6.4 4.2S1.6 8 1.6 8Z" />
      <circle cx="8" cy="8" r="1.9" />
    </>,
    p,
  );

export const Layers = (p: IconProps) =>
  svg(
    <>
      <path d="M8 1.9 14.4 5 8 8.1 1.6 5z" />
      <path d="m1.6 8 6.4 3.1L14.4 8" />
      <path d="m1.6 11 6.4 3.1L14.4 11" />
    </>,
    p,
  );

export const Check = (p: IconProps) => svg(<path d="m3.2 8.4 3.2 3.2 6.4-7" />, p);

export const Warn = (p: IconProps) =>
  svg(
    <>
      <path d="M8 2.4 14.6 13.6H1.4z" />
      <path d="M8 6.6v3.1M8 11.6v.1" />
    </>,
    p,
  );

export const Copy = (p: IconProps) =>
  svg(
    <>
      <rect x="5.4" y="5.4" width="8.2" height="8.2" rx="1.4" />
      <path d="M10.6 5.4V3.8a1.4 1.4 0 0 0-1.4-1.4H3.8a1.4 1.4 0 0 0-1.4 1.4v5.4a1.4 1.4 0 0 0 1.4 1.4h1.6" />
    </>,
    p,
  );

export const Import = (p: IconProps) =>
  svg(
    <>
      <path d="M8 2.4v7.4" />
      <path d="m4.9 6.8 3.1 3.1 3.1-3.1" />
      <path d="M2.6 12.4h10.8" />
    </>,
    p,
  );

export const Export = (p: IconProps) =>
  svg(
    <>
      <path d="M8 9.8V2.4" />
      <path d="m4.9 5.5 3.1-3.1 3.1 3.1" />
      <path d="M2.6 12.4h10.8" />
    </>,
    p,
  );

export const Trash = (p: IconProps) =>
  svg(
    <>
      <path d="M2.8 4.2h10.4" />
      <path d="M6.2 4.2V2.8h3.6v1.4" />
      <path d="M4.2 4.2 4.9 13.4h6.2l.7-9.2" />
    </>,
    p,
  );

export const Pencil = (p: IconProps) =>
  svg(
    <>
      <path d="M11.1 2.6 13.4 4.9 5.6 12.7 2.5 13.5l.8-3.1z" />
      <path d="m9.6 4.1 2.3 2.3" />
    </>,
    p,
  );

export const Send = (p: IconProps) =>
  svg(<path d="M14 2 2 6.6l4.7 1.9L8.6 13z" />, p);

export const Inbox = (p: IconProps) =>
  svg(
    <>
      <rect x="2" y="2.8" width="12" height="10.4" rx="1.6" />
      <path d="M2 9.4h3.2l1 1.6h3.6l1-1.6H14" />
    </>,
    p,
    16,
  );

export const Code = (p: IconProps) =>
  svg(
    <>
      <path d="m5.4 4.6-3.2 3.4 3.2 3.4" />
      <path d="m10.6 4.6 3.2 3.4-3.2 3.4" />
    </>,
    p,
  );

export const Branch = (p: IconProps) =>
  svg(
    <>
      <circle cx="4.5" cy="4" r="1.6" />
      <circle cx="4.5" cy="12" r="1.6" />
      <circle cx="11.5" cy="8" r="1.6" />
      <path d="M4.5 5.6v4.8M4.5 8h5.4" />
    </>,
    p,
  );

export const Beaker = (p: IconProps) =>
  svg(
    <>
      <path d="M6.4 2.2v4.1L2.9 12a1.3 1.3 0 0 0 1.1 2h8a1.3 1.3 0 0 0 1.1-2L9.6 6.3V2.2" />
      <path d="M5.4 2.2h5.2" />
    </>,
    p,
  );

export const Bolt = (p: IconProps) =>
  svg(<path d="M8.8 1.6 3.4 9.1h3.6l-.6 5.3 5.4-7.5H8.2z" />, p);

export const ArrowDown = (p: IconProps) =>
  svg(<path d="M8 3v10M4 9l4 4 4-4" />, p);

export const Plug = (p: IconProps) =>
  svg(
    <>
      <path d="M6 1.8v3.4M10 1.8v3.4" />
      <path d="M4.2 5.2h7.6v2.3a3.8 3.8 0 0 1-7.6 0z" />
      <path d="M8 11.3v2.9" />
    </>,
    p,
  );

export const Broadcast = (p: IconProps) =>
  svg(
    <>
      <circle cx="8" cy="8" r="1.6" />
      <path d="M4.8 4.8a4.5 4.5 0 0 0 0 6.4M11.2 4.8a4.5 4.5 0 0 1 0 6.4" />
      <path d="M2.6 2.6a7.6 7.6 0 0 0 0 10.8M13.4 2.6a7.6 7.6 0 0 1 0 10.8" />
    </>,
    p,
  );

export const Heart = (p: IconProps) =>
  svg(
    <path d="M8 13.2 2.9 8.4A3.2 3.2 0 0 1 7.4 3.6L8 4.2l.6-.6a3.2 3.2 0 0 1 4.5 4.8z" />,
    p,
  );
