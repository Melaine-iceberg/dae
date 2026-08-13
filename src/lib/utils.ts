import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB", "PB"] as const;

const BYTE_VALUE_FORMATTER = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 1 });

export function formatBytes(bytes: number): string {
  if (bytes <= 0) return "0 B";

  const unitIndex = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), BYTE_UNITS.length - 1);
  const value = bytes / 1024 ** unitIndex;

  return `${BYTE_VALUE_FORMATTER.format(value)} ${BYTE_UNITS[unitIndex]}`;
}
