import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { cn } from "@/lib/utils";

const appWindow = getCurrentWindow();

export function WindowControls() {
  const [maximized, setMaximized] = useState(false);
  const [focused, setFocused] = useState(true);

  useEffect(() => {
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
        aria-label="最小化"
        className={buttonClassName}
        onClick={() => void appWindow.minimize()}
        title="最小化"
        type="button"
      >
        <span aria-hidden="true" className="window-control-glyph">
          {"\uE921"}
        </span>
      </button>
      <button
        id="window-maximize"
        aria-label={maximized ? "向下还原" : "最大化"}
        className={buttonClassName}
        onClick={() => void appWindow.toggleMaximize()}
        title={maximized ? "向下还原" : "最大化"}
        type="button"
      >
        <span aria-hidden="true" className="window-control-glyph">
          {maximized ? "\uE923" : "\uE922"}
        </span>
      </button>
      <button
        aria-label="关闭"
        className={cn(buttonClassName, "hover:bg-destructive hover:text-white")}
        onClick={() => void appWindow.close()}
        title="关闭"
        type="button"
      >
        <span aria-hidden="true" className="window-control-glyph">
          {"\uE8BB"}
        </span>
      </button>
    </div>
  );
}
