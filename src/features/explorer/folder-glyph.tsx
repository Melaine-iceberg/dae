import { cn } from "@/lib/utils";

/**
 * The folder, drawn as three sheets of colour instead of one.
 *
 * Solar's closed folder is a single silhouette plus a short dash, so there is
 * nothing in it to give depth to; the open one has two layers but no paper. A
 * folder is the highest-volume glyph in the shell (38 of them in a full
 * listing) and the one thing in it drawn as an *object*, so it is the one place
 * where a little more colour structure pays for itself.
 *
 *   back   the tab and the rear wall, in the deep gold
 *   paper  a sheet showing between the two — open folders only
 *   front  the front wall, in the bright yellow people read as "folder"
 *   edge   a 1px light line along the front wall's top, which is what makes the
 *          front read as standing in front of the back
 *
 * Every layer is a flat fill taken from a token (`--folder`, `--folder-body`,
 * `--folder-edge`, `--folder-paper`) through `.folder-glyph` in App.css, never
 * a gradient: an `<linearGradient id>` repeated in each of the dozens of
 * inline SVGs a virtualised list mounts is a duplicate-id bug waiting for the
 * first one to unmount.
 *
 * The props match what Solar's glyphs take (`className`, `size`), because this
 * is a drop-in for them in `type-icon.tsx`, the sidebar's favourites and the
 * empty state.
 */

interface FolderGlyphProps {
  className?: string;
  size?: number | string;
  "aria-hidden"?: boolean;
}

function Frame({
  children,
  className,
  size = "1em",
  ...rest
}: FolderGlyphProps & { children: React.ReactNode }) {
  const px = typeof size === "number" ? `${size}px` : size;

  return (
    <svg
      aria-hidden={rest["aria-hidden"] ?? true}
      className={cn("folder-glyph shrink-0", className)}
      fill="none"
      height={px}
      viewBox="0 0 24 24"
      width={px}
      xmlns="http://www.w3.org/2000/svg"
    >
      {children}
    </svg>
  );
}

/** Closed: a rear wall with a tab, and the front wall over it. */
export function FolderGlyph(props: FolderGlyphProps) {
  return (
    <Frame {...props}>
      <path
        className="folder-back"
        d="M4.5 3h4.4c.55 0 1.07.22 1.46.6l1.1 1.1c.38.38.9.6 1.45.6H19.5A2.5 2.5 0 0 1 22 7.8V18.5A2.5 2.5 0 0 1 19.5 21h-15A2.5 2.5 0 0 1 2 18.5v-13A2.5 2.5 0 0 1 4.5 3Z"
      />
      <path
        className="folder-front"
        d="M2 10.5A2.5 2.5 0 0 1 4.5 8h15a2.5 2.5 0 0 1 2.5 2.5v8a2.5 2.5 0 0 1-2.5 2.5h-15A2.5 2.5 0 0 1 2 18.5v-8Z"
      />
      <path className="folder-edge" d="M4.5 8.5h15" />
    </Frame>
  );
}

/** Open: the rear wall, a sheet standing up in it, and a front wall that has
 *  swung forward — its top edge is slanted, which is what says "open". */
export function FolderOpenGlyph(props: FolderGlyphProps) {
  return (
    <Frame {...props}>
      <path
        className="folder-back"
        d="M4.5 3h4.4c.55 0 1.07.22 1.46.6l1.1 1.1c.38.38.9.6 1.45.6H19.5A2.5 2.5 0 0 1 22 7.8V10H2V5.5A2.5 2.5 0 0 1 4.5 3Z"
      />
      <path className="folder-paper" d="M5 7.5h14a1 1 0 0 1 1 1V12H4V8.5a1 1 0 0 1 1-1Z" />
      <path
        className="folder-front"
        d="M4.1 10.6A2.5 2.5 0 0 1 6.5 9h13.9a1.6 1.6 0 0 1 1.55 2.05l-1.7 6.9A2.5 2.5 0 0 1 17.8 20H4.5A2.5 2.5 0 0 1 2 17.5v-4.9c0-.9.8-1.55 1.7-1.55l.4-.45Z"
      />
      <path className="folder-edge" d="M6.6 9.5h13.7" />
    </Frame>
  );
}
