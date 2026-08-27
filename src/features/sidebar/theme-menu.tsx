import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
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
  value: ThemePreference;
}> = [
  { icon: SunIcon, value: "light" },
  { icon: MoonIcon, value: "dark" },
  { icon: MonitorIcon, value: "system" },
];

/**
 * Sidebar footer theme switcher (SKILL.md §12): light / dark / system with
 * the active mode reflected on the trigger icon.
 */
export function ThemeMenu() {
  const { t } = useTranslation("sidebar");
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
  const activeLabel = t(`theme.${preference}`);
  const title = t("theme.titleWith", { name: activeLabel });

  const updatePreference = (value: ThemePreference) => {
    setPreference(value);
    setThemePreference(value);
  };

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={title}
        className="flex size-7 items-center justify-center rounded-sm text-muted-foreground transition-colors outline-none hover:bg-accent/70 hover:text-foreground data-[popup-open]:bg-accent data-[popup-open]:text-foreground"
        title={title}
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
              {t(`theme.${option.value}`)}
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
