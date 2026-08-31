import type { CloudProviderKind } from "@/bindings";

/** Inline brand marks for the cloud account rows and the provider picker. */

export function GoogleDriveIcon({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      className={className}
      viewBox="0 0 87.3 78"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path
        d="m6.6 66.85 3.85 6.65c.8 1.4 1.95 2.5 3.3 3.3l13.75-23.8H0c0 1.55.4 3.1 1.2 4.5z"
        fill="#0066da"
      />
      <path
        d="M43.65 25 29.9 1.2c-1.35.8-2.5 1.9-3.3 3.3L1.2 48.5c-.8 1.4-1.2 2.95-1.2 4.5h27.5z"
        fill="#00ac47"
      />
      <path
        d="m73.55 76.8c1.35-.8 2.5-1.9 3.3-3.3l1.6-2.75 7.65-13.25c.8-1.4 1.2-2.95 1.2-4.5H59.85l5.85 11.5z"
        fill="#ea4335"
      />
      <path
        d="M43.65 25 29.9 1.2c1.35-.8 2.9-1.2 4.5-1.2h18.5c1.6 0 3.15.4 4.5 1.2z"
        fill="#00832d"
      />
      <path
        d="m59.8 53H27.5L13.75 76.8c1.35.8 2.9 1.2 4.5 1.2h50.8c1.6 0 3.15-.4 4.5-1.2z"
        fill="#2684fc"
      />
      <path
        d="m73.4 26.5-12.7-22c-.8-1.4-1.95-2.5-3.3-3.3L43.65 25l16.15 28h27.45c0-1.55-.4-3.1-1.2-4.5z"
        fill="#ffba00"
      />
    </svg>
  );
}

export function OneDriveIcon({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      className={className}
      viewBox="0 0 24 24"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path
        d="M2.25 15a4.5 4.5 0 0 0 4.5 4.5H18a3.75 3.75 0 0 0 1.332-7.257 3 3 0 0 0-3.758-3.848 5.25 5.25 0 0 0-10.233 2.33A4.502 4.502 0 0 0 2.25 15z"
        fill="#0364b8"
      />
    </svg>
  );
}

export function DropboxIcon({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      className={className}
      viewBox="0 0 24 24"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path
        d="M6 2 0 5.822 6 9.644l6-3.822L6 2zm12 0-6 3.822 6 3.822 6-3.822L18 2zM0 13.444l6 3.822 6-3.822-6-3.822-6 3.822zm18-3.822-6 3.822 6 3.822 6-3.822-6-3.822zM6 18.5l6 3.822 6-3.822-6-3.822L6 18.5z"
        fill="#0061ff"
      />
    </svg>
  );
}

export const CLOUD_PROVIDER_ICONS: Record<
  CloudProviderKind,
  (props: { className?: string }) => React.JSX.Element
> = {
  google_drive: GoogleDriveIcon,
  onedrive: OneDriveIcon,
  dropbox: DropboxIcon,
};
