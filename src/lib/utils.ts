import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

import { localeNumberFormat } from "@/i18n/format";

/**
 * `twMerge` with this theme's two custom scales declared.
 *
 * tailwind-merge resolves conflicts by CLASS GROUP, and it only knows the groups
 * its own config ships with — Tailwind's built-in scales. A class this theme
 * invents falls through every group it does not recognise and lands in the
 * catch-all, which is its own group that conflicts with nothing.
 *
 * That is invisible until a custom class shares a PREFIX with a built-in one,
 * which is what happened here and it broke visibly:
 *
 *     cn("px-2 pt-1.5 text-label text-muted-foreground")
 *
 * `text-muted-foreground` is a text COLOUR and is recognised. `text-label` is a
 * font SIZE from the `--text-*` ramp in App.css, is not a t-shirt size, and is
 * therefore classified as a colour too — so the two land in the same group and
 * the later one wins. `text-label` was silently DROPPED from every such call
 * site and the caption inherited whatever size it was sitting on: 16px where
 * the ramp asked for 11px. That is every menu group label, the context menu
 * and dropdown group captions, the file-list column headers, the command
 * palette's section captions, the recents group headings and the trash-view
 * group strip — seven call sites painting at the wrong size, with no error
 * anywhere, because the utility itself is fine. `text-label` only works when
 * it is the sole `text-*` class on the element, which is why the rows looked
 * right and the headings beside them did not.
 *
 * Declaring the ramp below puts the seven steps back in the font-size group,
 * where the later `text-*` colour no longer eats them.
 *
 * `rounded` is here for the same reason one step over: the radius scale is
 * `--radius-xs/sm/md/lg/xl`, and tailwind-merge knows `xs/sm/md/lg` as
 * t-shirt sizes but has no `xl`, so `rounded-xl` would not displace an
 * inherited `rounded-lg` — both would be emitted and plain CSS order would
 * pick the winner, which is not a thing to leave to chance.
 */
const twMerge = extendTailwindMerge({
  extend: {
    theme: {
      text: ["title", "lead", "body", "caption", "micro", "label", "nano"],
      radius: ["xs", "sm", "md", "lg", "xl"],
    },
  },
});

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB", "PB"] as const;

export function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";

  const unitIndex = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), BYTE_UNITS.length - 1);
  const value = bytes / 1024 ** unitIndex;

  const formatter = localeNumberFormat({ maximumFractionDigits: 1 });
  return `${formatter.format(value)} ${BYTE_UNITS[unitIndex]}`;
}
