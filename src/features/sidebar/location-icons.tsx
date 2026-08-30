import cloudLogo from "@/assets/cloud.svg";
import globeLogo from "@/assets/globe.svg";

/**
 * Colorful section-header icons for the location groups, in the same badge
 * family as the WSL logo. Fixed colors on purpose — they read as small brand
 * badges rather than theme-tinted glyphs.
 */

/** Hand-drawn disk badge: a slate rounded square with stacked white drives. */
export function DisksSectionIcon({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      className={className}
      viewBox="0 0 64 64"
      xmlns="http://www.w3.org/2000/svg"
    >
      <rect fill="#455A64" height="64" rx="14" width="64" />
      <rect fill="#fff" height="10" rx="5" width="36" x="14" y="13" />
      <rect fill="#fff" height="10" rx="5" width="36" x="14" y="27" />
      <rect fill="#fff" height="10" rx="5" width="36" x="14" y="41" />
      <circle cx="44" cy="18" fill="#FFC400" r="2.5" />
      <circle cx="44" cy="32" fill="#FFC400" r="2.5" />
      <circle cx="44" cy="46" fill="#FFC400" r="2.5" />
    </svg>
  );
}

/** Fluent color globe for the network storage section. */
export function NetworkSectionIcon({ className }: { className?: string }) {
  return <img alt="" aria-hidden="true" className={className} draggable={false} src={globeLogo} />;
}

/** Fluent color cloud for the cloud storage section. */
export function CloudSectionIcon({ className }: { className?: string }) {
  return <img alt="" aria-hidden="true" className={className} draggable={false} src={cloudLogo} />;
}
