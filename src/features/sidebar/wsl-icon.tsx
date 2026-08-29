import wslLogo from "@/assets/wsl.png";

/**
 * Official WSL logo for the section header. Fixed-color bitmap on purpose —
 * it reads as a small brand badge rather than a theme-tinted glyph.
 */
export function WslIcon({ className }: { className?: string }) {
  return <img alt="" aria-hidden="true" className={className} draggable={false} src={wslLogo} />;
}
