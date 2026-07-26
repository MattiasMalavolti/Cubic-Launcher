import { createSignal } from "solid-js";
import { pushUiError, setSettingsModalOpen } from "../store";
import { MaterialIcon } from "./icons";
import logoUrl from "../../assets/icon.png";

const isTauri = () => "__TAURI_INTERNALS__" in window;

export function Header() {
  const [maximized, setMaximized] = createSignal(false);

  const handleWindowAction = async (action: "minimize" | "toggle-maximize" | "close") => {
    if (!isTauri()) return;

    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const currentWindow = getCurrentWindow();

      if (action === "minimize") {
        await currentWindow.minimize();
        return;
      }

      if (action === "toggle-maximize") {
        if (await currentWindow.isMaximized()) {
          await currentWindow.unmaximize();
          setMaximized(false);
        } else {
          await currentWindow.maximize();
          setMaximized(true);
        }
        return;
      }

      await currentWindow.close();
    } catch (err) {
      pushUiError({ title: "Window control failed", message: "The desktop window action could not be completed.", detail: String(err), severity: "error", scope: "launch" });
    }
  };

  return (
    <header data-tauri-drag-region class="h-14 border-b border-borderColor bg-bgPanel flex items-center justify-between px-4 shrink-0 z-10 w-full">
      <div class="flex items-center gap-3">
        <img src={logoUrl} alt="Cubic Launcher" class="w-8 h-8 select-none" draggable={false} />
        <h1 class="font-semibold text-lg tracking-wide text-white/90">Cubic Launcher</h1>
      </div>
      <div class="flex items-center gap-4 text-textMuted">
        <button
          onClick={() => setSettingsModalOpen(true)}
          class="hover:text-white transition-colors duration-75"
          title="Settings"
        >
          <MaterialIcon name="settings" size="md" />
        </button>
        <button onClick={() => void handleWindowAction("minimize")} class="hover:text-white transition-colors duration-75" title="Minimize">
          <MaterialIcon name="remove" size="md" />
        </button>
        <button onClick={() => void handleWindowAction("toggle-maximize")} class="hover:text-white transition-colors duration-75" title={maximized() ? "Restore" : "Maximize"}>
          <MaterialIcon name={maximized() ? "filter_none" : "check_box_outline_blank"} size="md" />
        </button>
        <button onClick={() => void handleWindowAction("close")} class="hover:text-red-500 transition-colors duration-75" title="Close">
          <MaterialIcon name="close" size="md" />
        </button>
      </div>
    </header>
  );
}
