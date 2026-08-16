import { useEffect, useState } from "react";
import { MonitorIcon, MoonIcon, SunIcon } from "@phosphor-icons/react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  getStoredThemePreference,
  setThemePreference,
  type ThemePreference,
} from "@/lib/theme";

const THEME_OPTIONS: ReadonlyArray<{
  icon: typeof SunIcon;
  label: string;
  value: ThemePreference;
}> = [
  { icon: SunIcon, label: "浅色主题", value: "light" },
  { icon: MoonIcon, label: "深色主题", value: "dark" },
  { icon: MonitorIcon, label: "跟随系统", value: "system" },
];

/**
 * Sidebar footer theme switcher (SKILL.md §12): light / dark / system with
 * the active mode reflected on the trigger icon.
 */
export function ThemeMenu() {
  const [preference, setPreference] = useState<ThemePreference>(() =>
    getStoredThemePreference(),
  );

  useEffect(() => {
    const syncFromStorage = () => setPreference(getStoredThemePreference());
    window.addEventListener("app-theme-change", syncFromStorage);
    return () => window.removeEventListener("app-theme-change", syncFromStorage);
  }, []);

  const activeOption = THEME_OPTIONS.find((option) => option.value === preference);
  const ActiveIcon = activeOption?.icon ?? MonitorIcon;

  const updatePreference = (value: ThemePreference) => {
    setPreference(value);
    setThemePreference(value);
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={`主题：${activeOption?.label ?? "跟随系统"}`}
        className="flex size-7 items-center justify-center rounded-full text-muted-foreground transition-colors outline-none hover:bg-accent/70 hover:text-foreground data-[popup-open]:bg-accent data-[popup-open]:text-foreground"
        title={`主题：${activeOption?.label ?? "跟随系统"}`}
      >
        <ActiveIcon className="size-4" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-36">
        <DropdownMenuRadioGroup
          onValueChange={(value) => updatePreference(value as ThemePreference)}
          value={preference}
        >
          {THEME_OPTIONS.map((option) => (
            <DropdownMenuRadioItem key={option.value} value={option.value}>
              <option.icon />
              {option.label}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
