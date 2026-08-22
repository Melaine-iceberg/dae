import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import { getAppWindow } from "@/lib/app-window";

const appWindow = getAppWindow();

export function WindowControls() {
  const { t } = useTranslation("common");
  const [maximized, setMaximized] = useState(false);
  const [focused, setFocused] = useState(true);

  useEffect(() => {
    if (!appWindow) return;
    let disposed = false;
    const sync = () =>
      void appWindow.isMaximized().then((value) => {
        if (!disposed) setMaximized(value);
      });

    sync();
    const unlistenResizePromise = appWindow.onResized(sync);
    const unlistenFocusPromise = appWindow.onFocusChanged(({ payload }) => {
      if (!disposed) setFocused(payload);
    });

    return () => {
      disposed = true;
      void Promise.all([unlistenResizePromise, unlistenFocusPromise]).then((unlisten) => {
        unlisten.forEach((stopListening) => stopListening());
      });
    };
  }, []);

  const buttonClassName = cn(
    "flex h-full w-[46px] shrink-0 items-center justify-center transition-colors duration-75 hover:bg-accent hover:text-accent-foreground",
    focused ? "text-foreground" : "text-muted-foreground",
  );

  return (
    <div className="flex h-full shrink-0 items-stretch">
      <button
        aria-label={t("windowControls.minimize")}
        className={buttonClassName}
        onClick={() => void appWindow?.minimize()}
        title={t("windowControls.minimize")}
        type="button"
      >
        <span aria-hidden="true" className="window-control-glyph">
          {"\uE921"}
        </span>
      </button>
      <button
        id="window-maximize"
        aria-label={maximized ? t("windowControls.restore") : t("windowControls.maximize")}
        className={buttonClassName}
        onClick={() => void appWindow?.toggleMaximize()}
        title={maximized ? t("windowControls.restore") : t("windowControls.maximize")}
        type="button"
      >
        <span aria-hidden="true" className="window-control-glyph">
          {maximized ? "\uE923" : "\uE922"}
        </span>
      </button>
      <button
        aria-label={t("windowControls.close")}
        className={cn(buttonClassName, "hover:bg-destructive hover:text-white")}
        onClick={() => void appWindow?.close()}
        title={t("windowControls.close")}
        type="button"
      >
        <span aria-hidden="true" className="window-control-glyph">
          {"\uE8BB"}
        </span>
      </button>
    </div>
  );
}
