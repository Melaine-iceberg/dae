const darkModeMedia = window.matchMedia("(prefers-color-scheme: dark)");

export function applySystemTheme(): void {
  const dark = darkModeMedia.matches;
  document.documentElement.classList.toggle("dark", dark);
  document.documentElement.style.colorScheme = dark ? "dark" : "light";
}

export function watchSystemTheme(onChange: () => void): () => void {
  darkModeMedia.addEventListener("change", onChange);
  return () => darkModeMedia.removeEventListener("change", onChange);
}
